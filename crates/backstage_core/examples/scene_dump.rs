//! Prints the evaluated scene of a project at a time, e.g.
//! `cargo run -p backstage_core --example scene_dump -- samples/bounce.bs2d 0.5s`

use backstage_core::{DrawContent, RuntimeState, Time, evaluate, load};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(dir), time) = (args.next(), args.next()) else {
        eprintln!("usage: scene_dump <project.bs2d> [time, e.g. 0.5s]");
        std::process::exit(2);
    };
    let project = load(&PathBuf::from(dir))?;
    let now: Time = time.as_deref().unwrap_or("0s").parse()?;
    let scene = evaluate(&project, &RuntimeState::default(), now);
    println!("t = {now}: {} items", scene.items.len());
    for item in &scene.items {
        let comp = item.instance.iter().fold(project.root, |comp, node| {
            match &project.compositions[&comp].nodes[node].kind {
                backstage_core::NodeKind::Instance(i) => i.comp,
                _ => comp,
            }
        });
        let comp = &project.compositions[&comp];
        let path: Vec<_> =
            std::iter::once("stage".to_owned()).chain(item.instance.iter().map(|n| n.to_string())).collect();
        let pos = item.transform.transform_point2(backstage_core::Vec2::ZERO);
        let content = match item.content {
            DrawContent::Shape(shape) => match &comp.nodes[&item.node].kind {
                backstage_core::NodeKind::Flipbook(drawings) => {
                    let index = drawings.iter().position(|d| std::ptr::eq(&d.shape, shape)).unwrap_or(0);
                    format!("flipbook drawing {index} of {}", drawings.len())
                }
                _ => format!("shape ({} paths)", shape.paths.len()),
            },
            DrawContent::Bitmap(asset) => format!("bitmap {asset}"),
        };
        println!(
            "  {:<28} {:<10} at ({:7.2}, {:7.2})  opacity {:.2}  {}",
            format!("{}/{}", comp.name, comp.nodes[&item.node].name),
            format!("[{}]", path.len() - 1),
            pos.x,
            pos.y,
            item.opacity,
            content
        );
    }
    Ok(())
}
