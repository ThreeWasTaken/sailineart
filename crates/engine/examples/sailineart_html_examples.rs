use photocraft_engine::Session;
use serde_json::json;
use std::path::{Path, PathBuf};

fn new_doc(name: &str, width: u32, height: u32) -> Result<Session, Box<dyn std::error::Error>> {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":width,"height":height,"background":"#ffffff","name":name}))?;
    Ok(session)
}

fn create(
    session: &mut Session,
    points: &[[f64; 2]],
    width: f32,
    color: &str,
    factors: &[f32],
    straight: bool,
) -> Result<(u64, u64), Box<dyn std::error::Error>> {
    let result = session.execute(
        "linework.create",
        json!({"points":points,"width":width,"color":color,"widthFactors":factors,"straight":straight,"interpolation":"saiSimplified"}),
    )?;
    let layer = result["layer"].as_u64().ok_or("missing Linework layer id")?;
    let path = match &session.active().ok_or("missing document")?.doc.layer(photocraft_doc::LayerId(layer)).ok_or("missing Linework layer")?.content {
        photocraft_doc::LayerContent::Linework(linework) => linework.paths.last().ok_or("missing Linework path")?.id,
        _ => return Err("expected Linework layer".into()),
    };
    Ok((layer, path))
}

fn export(session: &Session, folder: &Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let document = &session.active().ok_or("missing document")?.doc;
    let image = photocraft_io::export(document, "png", &Default::default())?;
    std::fs::write(folder.join(name), image.bytes)?;
    Ok(())
}

fn export_endpoints(folder: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut session = new_doc("SaiLineArt — terminal tools", 520, 260)?;
    let (layer, a) = create(&mut session, &[[55.0, 205.0], [220.0, 100.0]], 13.0, "#222733", &[1.0, 1.0], false)?;
    let (_, b) = create(&mut session, &[[260.0, 75.0], [465.0, 50.0]], 13.0, "#222733", &[1.0, 1.0], false)?;
    export(&session, folder, "combine-before.png")?;
    let point_a = match &session.active().ok_or("missing document")?.doc.layer(photocraft_doc::LayerId(layer)).ok_or("missing Linework layer")?.content {
        photocraft_doc::LayerContent::Linework(lw) => lw.paths.iter().find(|p| p.id == a).and_then(|p| p.points.last()).ok_or("missing endpoint")?.id,
        _ => return Err("expected Linework layer".into()),
    };
    let point_b = match &session.active().ok_or("missing document")?.doc.layer(photocraft_doc::LayerId(layer)).ok_or("missing Linework layer")?.content {
        photocraft_doc::LayerContent::Linework(lw) => lw.paths.iter().find(|p| p.id == b).and_then(|p| p.points.first()).ok_or("missing endpoint")?.id,
        _ => return Err("expected Linework layer".into()),
    };
    session.execute("linework.combinePaths", json!({"layer":layer,"pathA":a,"pointA":point_a,"pathB":b,"pointB":point_b}))?;
    export(&session, folder, "combine-after.png")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("docs/images/sailineart"));
    std::fs::create_dir_all(&folder)?;

    let mut session = new_doc("SaiCurve", 520, 260)?;
    create(&mut session, &[[55.0, 205.0], [240.0, 75.0], [465.0, 190.0]], 10.0, "#20242b", &[1.0; 3], false)?;
    export(&session, &folder, "tool-curve.png")?;

    let mut session = new_doc("SaiLineArt — SaiLine tool", 520, 260)?;
    create(&mut session, &[[55.0, 200.0], [215.0, 55.0], [320.0, 200.0], [465.0, 75.0]], 10.0, "#20242b", &[1.0; 4], true)?;
    export(&session, &folder, "tool-line.png")?;

    let mut session = new_doc("SaiPressure", 520, 260)?;
    create(&mut session, &[[50.0, 185.0], [155.0, 70.0], [260.0, 155.0], [365.0, 70.0], [470.0, 185.0]], 23.0, "#20242b", &[0.0, 0.75, 2.0, 0.75, 0.0], false)?;
    export(&session, &folder, "tool-pressure.png")?;

    let mut session = new_doc("SaiWeight", 520, 260)?;
    create(&mut session, &[[55.0, 85.0], [240.0, 40.0], [465.0, 85.0]], 3.0, "#20242b", &[1.0; 3], false)?;
    create(&mut session, &[[55.0, 210.0], [240.0, 165.0], [465.0, 210.0]], 18.0, "#20242b", &[1.0; 3], false)?;
    export(&session, &folder, "tool-weight.png")?;

    let mut session = new_doc("SaiColor", 520, 260)?;
    let (layer, red) = create(&mut session, &[[50.0, 195.0], [240.0, 65.0], [470.0, 180.0]], 12.0, "#e53935", &[1.0; 3], false)?;
    create(&mut session, &[[50.0, 220.0], [240.0, 100.0], [470.0, 205.0]], 7.0, "#16a085", &[1.0; 3], false)?;
    session.execute("linework.setPathColor", json!({"layer":layer,"path":red,"color":"#2867d7"}))?;
    export(&session, &folder, "tool-color.png")?;

    let mut session = new_doc("SaiPointed", 520, 300)?;
    create(&mut session, &[[55.0, 115.0], [260.0, 55.0], [465.0, 115.0]], 11.0, "#20242b", &[1.0; 3], false)?;
    let (layer, path) = create(&mut session, &[[55.0, 255.0], [260.0, 190.0], [465.0, 255.0]], 11.0, "#20242b", &[1.0; 3], false)?;
    let point = match &session.active().ok_or("missing document")?.doc.layer(photocraft_doc::LayerId(layer)).ok_or("missing Linework layer")?.content {
        photocraft_doc::LayerContent::Linework(lw) => lw.paths.iter().find(|p| p.id == path).and_then(|p| p.points.get(1)).ok_or("missing middle point")?.id,
        _ => return Err("expected Linework layer".into()),
    };
    session.execute("linework.setPointPointed", json!({"layer":layer,"path":path,"point":point,"pointed":true}))?;
    export(&session, &folder, "tool-pointed.png")?;

    export_endpoints(&folder)?;
    println!("SaiLineArt example PNGs written to {}", folder.display());
    Ok(())
}
