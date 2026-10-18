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
