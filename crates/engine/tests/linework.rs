//! SaiLine command, persistence and history.

use photocraft_doc::LayerContent;
use photocraft_engine::Session;
use photocraft_format::{SaveOptions, load_from_bytes, save_to_bytes};
use serde_json::{Value, json};

fn command_is_listed(s: &mut Session, id: &str) -> bool {
    s.execute("command.list", json!({})).unwrap().as_array().is_some_and(|rows| rows.iter().any(|row| row.get("id") == Some(&Value::String(id.into()))))
}

#[test]
fn creates_a_dedicated_linework_layer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 96})).unwrap();
    let before = s.active().unwrap().doc.layers.len();
    let result = s
        .execute("linework.create", json!({"points": [[10.0, 12.0], [40.0, 70.0], [100.0, 24.0]], "width": 5.0, "color": "#3366cc", "name": "SaiLine test"}))
        .unwrap();
    let id = result["layer"].as_u64().unwrap();
    assert_eq!(result["kind"], "linework");
    assert_eq!(result["points"].as_array().unwrap().len(), 3);
    assert!(command_is_listed(&mut s, "linework.create"));
    assert_eq!(s.journal.last().map(|entry| entry.0.as_str()), Some("linework.create"));

    let state = s.active().unwrap();
    assert_eq!(state.doc.layers.len(), before + 1);
    let layer = state.doc.layer(photocraft_doc::LayerId(id)).unwrap();
    let LayerContent::Linework(linework) = &layer.content else { panic!("linework must use its dedicated layer model") };
    assert_eq!(linework.paths.len(), 1);
    assert_eq!(linework.paths[0].width, 5.0);
    assert_eq!(linework.paths[0].points[1].position, [40.0, 70.0]);
}

#[test]
fn recoloring_a_path_preserves_its_point_properties_and_is_undoable() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":96,"height":64})).unwrap();
    let created = s.execute("linework.create", json!({"points":[[8,12],[44,28],[80,12]],"width":7,"color":"#112233","widthFactors":[0.25,1.5,2.0]})).unwrap();
    let layer = photocraft_doc::LayerId(created["layer"].as_u64().unwrap());
    let path = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths[0].clone(),
        _ => panic!(),
    };
    s.execute("linework.setPathColor", json!({"layer":layer.0,"path":path.id,"color":"#d04080"})).unwrap();
    let colored = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths[0].clone(),
        _ => panic!(),
    };
    assert_eq!(colored.color.to_rgb(), [0xd0 as f32 / 255.0, 0x40 as f32 / 255.0, 0x80 as f32 / 255.0]);
    assert_eq!(colored.points.iter().map(|p| p.width_factor).collect::<Vec<_>>(), vec![0.25, 1.5, 2.0]);
    assert_eq!(colored.width, path.width);
    assert_eq!(colored.points.iter().map(|p| p.position).collect::<Vec<_>>(), path.points.iter().map(|p| p.position).collect::<Vec<_>>());
    assert!(s.undo());
    let restored = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths[0].clone(),
        _ => panic!(),
    };
    assert_eq!(restored.color, path.color);
    assert!(s.redo());
    let bytes = save_to_bytes(&s.active().unwrap().doc, &SaveOptions::default()).unwrap();
    let loaded = load_from_bytes(&bytes).unwrap();
    let persisted = match &loaded.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths[0].clone(),
        _ => panic!(),
    };
    assert_eq!(persisted.color, colored.color);
    assert_eq!(persisted.points, colored.points);
}

#[test]
fn sai_weight_changes_only_the_base_path_width_and_is_undoable() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":96,"height":64})).unwrap();
    let created = s.execute("linework.create", json!({"points":[[8,12],[44,28],[80,12]],"width":7,"widthFactors":[0.25,1.5,2.0]})).unwrap();
    let layer = photocraft_doc::LayerId(created["layer"].as_u64().unwrap());
    let path_id = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths[0].id,
        _ => panic!(),
    };
    s.execute("linework.setPathWidth", json!({"layer":layer.0,"path":path_id,"width":30.0})).unwrap();
    let read_path = |s: &Session| match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths.iter().find(|p| p.id == path_id).unwrap().clone(),
        _ => panic!(),
    };
    let widened = read_path(&s);
    assert_eq!(widened.width, 30.0);
    assert_eq!(widened.points.iter().map(|p| p.width_factor).collect::<Vec<_>>(), vec![0.25, 1.5, 2.0]);
    assert!(s.undo());
    assert_eq!(read_path(&s).width, 7.0);
    assert!(s.redo());
    assert_eq!(read_path(&s).width, 30.0);
    assert!(s.execute("linework.setPathWidth", json!({"layer":layer.0,"path":path_id,"width":31.0})).is_err());
}

#[test]
fn deleting_an_internal_segment_splits_the_path_and_preserves_control_points() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":128,"height":96})).unwrap();
    let created = s
        .execute(
            "linework.create",
            json!({"points":[[8,12],[36,20],[64,28],[92,36]],"width":6,"color":"#427ab5","widthFactors":[0.5,0.75,1.5,2.0],"straight":true}),
        )
        .unwrap();
    let layer = photocraft_doc::LayerId(created["layer"].as_u64().unwrap());
    let original = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths[0].clone(),
        _ => panic!(),
    };
    let split = s.execute("linework.deleteSegment", json!({"layer":layer.0,"path":original.id,"segment":1})).unwrap();
    assert_ne!(split["newPath"], Value::Null);
    let paths = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths.clone(),
        _ => panic!(),
    };
    assert_eq!(paths.len(), 2);
    let first = paths.iter().find(|path| path.id == original.id).unwrap();
    let second = paths.iter().find(|path| Some(path.id) == split["newPath"].as_u64()).unwrap();
    assert_eq!(first.points, original.points[..2]);
    assert_eq!(second.points, original.points[2..]);
    assert_eq!(first.width, original.width);
    assert_eq!(second.width, original.width);
    assert_eq!(first.color, original.color);
    assert_eq!(second.color, original.color);
    assert!(first.points.iter().all(|point| point.pointed));
    assert!(second.points.iter().all(|point| point.pointed));
    assert!(s.undo());
    let restored = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths.clone(),
        _ => panic!(),
    };
    assert_eq!(restored, vec![original]);
    assert!(s.redo());
    let redone = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths.len(),
        _ => panic!(),
    };
    assert_eq!(redone, 2);
}

#[test]
fn deleting_an_end_segment_removes_its_endpoint_and_a_two_point_path() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":96,"height":64})).unwrap();
    let created = s.execute("linework.create", json!({"points":[[8,12],[36,20],[64,28]],"width":4})).unwrap();
    let layer = photocraft_doc::LayerId(created["layer"].as_u64().unwrap());
    let (path_id, first_point) = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => (lw.paths[0].id, lw.paths[0].points[0].id),
        _ => panic!(),
    };
    let deleted = s.execute("linework.deleteSegment", json!({"layer":layer.0,"path":path_id,"segment":0})).unwrap();
    assert_eq!(deleted["removedPoint"].as_u64(), Some(first_point));
    let remaining = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths[0].points.len(),
        _ => panic!(),
    };
    assert_eq!(remaining, 2);
    s.execute("linework.deleteSegment", json!({"layer":layer.0,"path":path_id,"segment":0})).unwrap();
    let paths = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(lw) => lw.paths.len(),
        _ => panic!(),
    };
    assert_eq!(paths, 0);
}

#[test]
fn combining_two_linework_endpoints_keeps_point_properties_and_is_undoable() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":96,"height":64})).unwrap();
    let a = s.execute("linework.create", json!({"points":[[8,12],[30,20]],"width":5,"widthFactors":[0.5,1.5]})).unwrap();
    s.execute("linework.create", json!({"points":[[30,20],[80,12]],"width":5,"widthFactors":[1.25,0.75]})).unwrap();
    let layer = photocraft_doc::LayerId(a["layer"].as_u64().unwrap());
    let (pa, pb, join_a, join_b) = {
        let LayerContent::Linework(lw) = &s.active().unwrap().doc.layer(layer).unwrap().content else { panic!() };
        let pa = lw.paths.iter().find(|p| p.id == lw.paths[0].id).unwrap();
        let pb = lw.paths.iter().find(|p| p.id != pa.id).unwrap();
        (pa.id, pb.id, pa.points.last().unwrap().id, pb.points.first().unwrap().id)
    };
    s.execute("linework.combinePaths", json!({"layer":layer.0,"pathA":pa,"pointA":join_a,"pathB":pb,"pointB":join_b})).unwrap();
    let LayerContent::Linework(lw) = &s.active().unwrap().doc.layer(layer).unwrap().content else { panic!() };
    assert_eq!(lw.paths.len(), 1);
    assert_eq!(lw.paths[0].points.len(), 3);
    assert_eq!(lw.paths[0].points.iter().map(|p| p.width_factor).collect::<Vec<_>>(), vec![0.5, 1.5, 0.75]);
    assert!(s.undo());
    let LayerContent::Linework(lw) = &s.active().unwrap().doc.layer(layer).unwrap().content else { panic!() };
    assert_eq!(lw.paths.len(), 2);
}

#[test]
fn creating_again_on_active_linework_reuses_the_same_layer() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 128, "height": 96})).unwrap();
    let first = s.execute("linework.create", json!({"points": [[10, 12], [40, 70]], "width": 3})).unwrap();
    let second = s.execute("linework.create", json!({"points": [[60, 20], [100, 40]], "width": 3})).unwrap();
    assert_eq!(first["layer"], second["layer"]);
    let layer = s.active().unwrap().doc.layer(photocraft_doc::LayerId(first["layer"].as_u64().unwrap())).unwrap();
    let LayerContent::Linework(linework) = &layer.content else { panic!("linework layer expected") };
    assert_eq!(linework.paths.len(), 2);
    assert!(linework.cache.is_some());
}

#[test]
fn linework_roundtrips_through_pcraft_and_history() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let result = s.execute("linework.create", json!({"points": [[4, 8], [20, 40], [56, 12]], "width": 2, "color": [1, 0.25, 0, 1]})).unwrap();
    let id = photocraft_doc::LayerId(result["layer"].as_u64().unwrap());
    let doc = s.active().unwrap().doc.clone();
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    let loaded = load_from_bytes(&bytes).unwrap();
    let loaded_layer = loaded.layer(id).unwrap();
    let original_layer = doc.layer(id).unwrap();
    assert_eq!(loaded_layer.content, original_layer.content);
    assert_eq!(loaded_layer.name, original_layer.name);
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(id).is_none());
    assert!(s.redo());
    assert!(s.active().unwrap().doc.layer(id).is_some());
    assert_eq!(load_from_bytes(&bytes).unwrap().layer(id).unwrap().content, s.active().unwrap().doc.layer(id).unwrap().content);
}

#[test]
fn linework_rejects_invalid_input_without_changing_history() {
    let mut s = Session::new();
    assert!(s.execute("linework.create", json!({"points": [[0, 0], [1, 1]]})).is_err());
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    let before_layers = s.active().unwrap().doc.layers.len();
    let before_history = s.active().unwrap().history.entries();
    for params in [
        json!({"points": []}),
        json!({"points": [[0, 0]]}),
        json!({"points": [[0, 0], [1, 1]], "width": 0}),
        json!({"points": [[0, 0], [1, 1]], "color": "not-a-color"}),
    ] {
        assert!(s.execute("linework.create", params).is_err());
        assert_eq!(s.active().unwrap().doc.layers.len(), before_layers);
        assert_eq!(s.active().unwrap().history.entries(), before_history);
    }
}

#[test]
fn linework_point_drag_coalesces_micro_moves_into_one_undo_step() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let result = s.execute("linework.create", json!({"points": [[4, 8], [20, 40]], "width": 2})).unwrap();
    let layer = result["layer"].as_u64().unwrap();
    let (path, point) = {
        let linework = match &s.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content {
            LayerContent::Linework(linework) => linework,
            _ => panic!("linework layer expected"),
        };
        (linework.paths[0].id, linework.paths[0].points[0].id)
    };
    let before = s.active().unwrap().history.entries().len();

    for x in [10.0, 12.0, 16.0] {
        s.execute("linework.movePoint", json!({"layer": layer, "path": path, "point": point, "x": x, "y": 8.0, "coalesce": "sailine-move:test"})).unwrap();
    }
    assert_eq!(s.active().unwrap().history.entries().len(), before + 1);
    let point_position = |s: &Session| {
        let layer = s.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap();
        let LayerContent::Linework(linework) = &layer.content else { panic!("linework layer expected") };
        linework.paths[0].points.iter().find(|candidate| candidate.id == point).map(|candidate| candidate.position)
    };
    assert_eq!(point_position(&s), Some([16.0, 8.0]));
    assert!(s.undo());
    assert_eq!(point_position(&s), Some([4.0, 8.0]));
    assert!(s.redo());
    assert_eq!(point_position(&s), Some([16.0, 8.0]));
}

#[test]
fn linework_point_width_is_persisted_and_rerenders_the_outline() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let result = s.execute("linework.create", json!({"points": [[4, 8], [20, 40], [56, 12]], "width": 4, "widthFactors": [0.5, 2.0, 1.0]})).unwrap();
    let layer = photocraft_doc::LayerId(result["layer"].as_u64().unwrap());
    let (path, point) = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => (linework.paths[0].id, linework.paths[0].points[1].id),
        _ => panic!("linework layer expected"),
    };
    s.execute("linework.setPointWidth", json!({"layer": layer.0, "path": path, "point": point, "factor": 2.5})).unwrap();
    let LayerContent::Linework(linework) = &s.active().unwrap().doc.layer(layer).unwrap().content else { panic!("linework layer expected") };
    assert_eq!(linework.paths[0].points.iter().map(|point| point.width_factor).collect::<Vec<_>>(), vec![0.5, 2.5, 1.0]);
    assert!(linework.cache.is_some());
}

#[test]
fn pressure_range_insert_undo_redo_and_serialization_preserve_point_values() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 64})).unwrap();
    let result = s.execute("linework.create", json!({"points": [[8, 24], [88, 24]], "width": 8, "widthFactors": [0.0, 2.0]})).unwrap();
    let layer = photocraft_doc::LayerId(result["layer"].as_u64().unwrap());
    let path = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => linework.paths[0].id,
        _ => panic!("linework layer expected"),
    };
    let inserted = s.execute("linework.insertPoint", json!({"layer": layer.0, "path": path, "index": 1, "x": 40, "y": 24, "factor": 1.25})).unwrap();
    let inserted_id = inserted["point"].as_u64().unwrap();
    let values = |s: &Session| match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => linework.paths[0].points.iter().map(|point| (point.id, point.width_factor)).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert_eq!(values(&s).iter().map(|(_, factor)| *factor).collect::<Vec<_>>(), vec![0.0, 1.25, 2.0]);
    assert!(s.undo());
    assert_eq!(values(&s).iter().map(|(_, factor)| *factor).collect::<Vec<_>>(), vec![0.0, 2.0]);
    assert!(s.redo());
    assert!(values(&s).iter().any(|(id, factor)| *id == inserted_id && *factor == 1.25));
    let bytes = save_to_bytes(&s.active().unwrap().doc, &SaveOptions::default()).unwrap();
    let loaded = load_from_bytes(&bytes).unwrap();
    let LayerContent::Linework(loaded_linework) = &loaded.layer(layer).unwrap().content else { panic!("linework layer expected") };
    assert_eq!(loaded_linework.paths[0].points.iter().map(|point| point.width_factor).collect::<Vec<_>>(), vec![0.0, 1.25, 2.0]);
    assert!(s.execute("linework.insertPoint", json!({"layer": layer.0, "path": path, "index": 1, "x": 32, "y": 24, "factor": 2.01})).is_err());
}

#[test]
fn batch_pressure_and_deletion_are_atomic_and_undoable() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 160, "height": 100})).unwrap();
    let first = s.execute("linework.create", json!({"points": [[10, 20], [50, 70], [90, 20]], "width": 4})).unwrap();
    let second = s.execute("linework.create", json!({"points": [[20, 80], [80, 80]], "width": 4})).unwrap();
    let layer = photocraft_doc::LayerId(first["layer"].as_u64().unwrap());
    let (path_a, points_a, path_b, points_b) = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => (
            linework.paths[0].id,
            linework.paths[0].points.iter().map(|p| p.id).collect::<Vec<_>>(),
            linework.paths[1].id,
            linework.paths[1].points.iter().map(|p| p.id).collect::<Vec<_>>(),
        ),
        _ => panic!("linework layer expected"),
    };
    assert_eq!(first["layer"], second["layer"]);
    let target = |path, point, factor| json!({"layer": layer.0, "path": path, "point": point, "factor": factor});
    let width_before = s.active().unwrap().history.entries().len();
    s.execute("linework.setPointWidths", json!({"points": [target(path_a, points_a[0], 0.5), target(path_b, points_b[1], 1.75)]})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), width_before + 1);
    let factors = |s: &Session| match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => {
            linework.paths.iter().map(|path| path.points.iter().map(|p| p.width_factor).collect::<Vec<_>>()).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    assert_eq!(factors(&s), vec![vec![0.5, 1.0, 1.0], vec![1.0, 1.75]]);
    assert!(s.undo());
    assert_eq!(factors(&s), vec![vec![1.0; 3], vec![1.0; 2]]);
    assert!(s.redo());
    let delete_before = s.active().unwrap().history.entries().len();
    let id = |path, point| json!({"layer": layer.0, "path": path, "point": point});
    s.execute("linework.deletePoints", json!({"points": [id(path_a, points_a[1]), id(path_b, points_b[0]), id(path_b, points_b[1])]})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), delete_before + 1);
    let LayerContent::Linework(linework) = &s.active().unwrap().doc.layer(layer).unwrap().content else { panic!("linework layer expected") };
    assert_eq!(linework.paths.len(), 1);
    assert_eq!(linework.paths[0].points.len(), 2);
    assert!(s.undo());
    assert_eq!(factors(&s), vec![vec![0.5, 1.0, 1.0], vec![1.0, 1.75]]);
    assert!(s.redo());
    assert!(s.execute("linework.deletePoints", json!({"points": [id(path_a, points_a[0]), id(path_a, 999999)]})).is_err());
    let LayerContent::Linework(linework) = &s.active().unwrap().doc.layer(layer).unwrap().content else { panic!("linework layer expected") };
    assert_eq!(linework.paths[0].points.len(), 2);
}

#[test]
fn interpolation_preview_switches_geometry_without_history_or_document_changes() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 96})).unwrap();
    let result = s.execute("linework.create", json!({"points": [[5, 5], [25, 60], [80, 10]], "width": 4})).unwrap();
    let layer = photocraft_doc::LayerId(result["layer"].as_u64().unwrap());
    let points_before = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => linework.paths[0].points.clone(),
        _ => panic!("linework layer expected"),
    };
    let history_before = s.active().unwrap().history.entries().len();
    let journal_before = s.journal.len();
    let was_clean = s.active().unwrap().saved_revision == s.active().unwrap().revision;
    let legacy_cache = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => linework.cache.clone(),
        _ => None,
    };

    s.execute("linework.setInterpolation", json!({"interpolation": "saiOptimized"})).unwrap();
    let state = s.active().unwrap();
    assert_eq!(state.history.entries().len(), history_before);
    assert_eq!(s.journal.len(), journal_before);
    assert_eq!(state.saved_revision == state.revision, was_clean, "view-only preview must preserve document dirty state");
    let (points_after, optimized_cache) = match &state.doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => (linework.paths[0].points.clone(), linework.cache.clone()),
        _ => panic!("linework layer expected"),
    };
    assert_eq!(points_after, points_before);
    assert_ne!(optimized_cache, legacy_cache, "model switch should immediately rebuild the rendered geometry");
}

#[test]
fn selected_interpolation_is_used_by_edits_and_survives_undo_redo_and_save() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 96})).unwrap();
    let result = s.execute("linework.create", json!({"points": [[5, 5], [25, 60], [80, 10]], "width": 4, "interpolation": "saiOptimized"})).unwrap();
    let layer = photocraft_doc::LayerId(result["layer"].as_u64().unwrap());
    let (path, point) = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => (linework.paths[0].id, linework.paths[0].points[1].id),
        _ => panic!("linework layer expected"),
    };
    s.execute("linework.movePoint", json!({"layer": layer.0, "path": path, "point": point, "x": 30, "y": 55, "interpolation": "saiOptimized"})).unwrap();
    let moved_cache = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => linework.cache.clone(),
        _ => None,
    };
    assert!(s.undo());
    assert!(s.redo());
    let redone_cache = match &s.active().unwrap().doc.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => linework.cache.clone(),
        _ => None,
    };
    assert_eq!(redone_cache, moved_cache);
    let bytes = save_to_bytes(&s.active().unwrap().doc, &SaveOptions::default()).unwrap();
    let loaded = load_from_bytes(&bytes).unwrap();
    let loaded_linework = match &loaded.layer(layer).unwrap().content {
        LayerContent::Linework(linework) => linework,
        _ => panic!("linework layer expected"),
    };
    assert_eq!(loaded_linework.paths[0].points[1].position, [30.0, 55.0]);
    assert_eq!(loaded_linework.cache, redone_cache);
}

#[test]
fn incremental_linework_cache_matches_full_rebuild_after_drag_and_pressure() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 256, "height": 192})).unwrap();
    let layer = s.execute("linework.create", json!({"points": [[8, 15], [70, 90], [120, 20]], "width": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("linework.create", json!({"points": [[25, 130], [95, 70], [180, 140]], "width": 7})).unwrap();
    s.execute("linework.create", json!({"points": [[130, 18], [220, 90], [230, 160]], "width": 5})).unwrap();
    let ids = || {
        let LayerContent::Linework(linework) = &s.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!() };
        linework.paths.iter().map(|path| (path.id, path.points[1].id)).collect::<Vec<_>>()
    };
    let ids = ids();
    let assert_rebuild_equal = |s: &mut Session| {
        let before = {
            let LayerContent::Linework(linework) = &s.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!() };
            linework.cache.clone().unwrap()
        };
        s.execute("linework.setInterpolation", json!({"interpolation": "saiSimplified"})).unwrap();
        let LayerContent::Linework(linework) = &s.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!() };
        let bounds = s.active().unwrap().doc.bounds();
        assert_eq!(before.read_region(bounds), linework.cache.as_ref().unwrap().read_region(bounds));
    };
    s.execute("linework.movePoint", json!({"layer": layer, "path": ids[1].0, "point": ids[1].1, "x": 105, "y": 60})).unwrap();
    assert_rebuild_equal(&mut s);
    s.execute("linework.setPointWidth", json!({"layer": layer, "path": ids[0].0, "point": ids[0].1, "factor": 2.0})).unwrap();
    assert_rebuild_equal(&mut s);
    s.execute(
        "linework.movePoints",
        json!({"points": [
            {"layer": layer, "path": ids[0].0, "point": ids[0].1, "x": 76, "y": 96},
            {"layer": layer, "path": ids[2].0, "point": ids[2].1, "x": 205, "y": 82}
        ]}),
    )
    .unwrap();
    assert_rebuild_equal(&mut s);
    s.execute("layer.translate", json!({"layer": layer, "dx": 3, "dy": -2})).unwrap();
    s.execute("linework.setPointWidth", json!({"layer": layer, "path": ids[1].0, "point": ids[1].1, "factor": 1.5})).unwrap();
    assert_rebuild_equal(&mut s);
    let cache = {
        let LayerContent::Linework(linework) = &s.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!() };
        linework.cache.clone()
    };
    assert!(s.undo());
    assert!(s.redo());
    let LayerContent::Linework(linework) = &s.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!() };
    assert_eq!(linework.cache, cache);
}

#[test]
fn path_raster_snapshots_survive_undo_redo_and_release_with_history() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 128, "height": 64})).unwrap();
    let layer = session.execute("linework.create", json!({"points": [[8, 32], [120, 32]], "width": 8})).unwrap()["layer"].as_u64().unwrap();
    let (path_id, point_id, original_cache) = {
        let LayerContent::Linework(linework) = &session.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!() };
        let path = &linework.paths[0];
        (path.id, path.points[0].id, linework.path_caches.get(&path.id).unwrap().clone())
    };
    let original_pixels = original_cache.pixels.clone();
    let original_weak = std::sync::Arc::downgrade(&original_cache.pixels);

    session.execute("linework.setPointWidth", json!({"layer": layer, "path": path_id, "point": point_id, "factor": 2.0})).unwrap();
    let changed_cache = {
        let LayerContent::Linework(linework) = &session.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!() };
        linework.path_caches.get(&path_id).unwrap().clone()
    };
    let changed_pixels = changed_cache.pixels.clone();
    let changed_weak = std::sync::Arc::downgrade(&changed_cache.pixels);
    assert!(!std::sync::Arc::ptr_eq(&original_pixels, &changed_pixels));
    assert!(session.undo());
    {
        let LayerContent::Linework(linework) = &session.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!() };
        assert!(std::sync::Arc::ptr_eq(&linework.path_caches.get(&path_id).unwrap().pixels, &original_pixels));
        assert_eq!(linework.path_caches.get(&path_id).unwrap().pixels.as_ref(), original_pixels.as_ref());
    }
    assert!(session.redo());
    {
        let LayerContent::Linework(linework) = &session.active().unwrap().doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!() };
        assert!(std::sync::Arc::ptr_eq(&linework.path_caches.get(&path_id).unwrap().pixels, &changed_pixels));
        assert_eq!(linework.path_caches.get(&path_id).unwrap().pixels.as_ref(), changed_pixels.as_ref());
    }

    drop(original_cache);
    drop(changed_cache);
    drop(original_pixels);
    drop(changed_pixels);
    drop(session);
    assert!(original_weak.upgrade().is_none());
    assert!(changed_weak.upgrade().is_none());
}
