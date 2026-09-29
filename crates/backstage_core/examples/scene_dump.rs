//! Prints the evaluated scene of a project at a time, e.g.
//! `cargo run -p backstage_core --example scene_dump -- samples/bounce.bs2d 0.5s`
//!
//! `--crossfade <animation>@<start>/<duration>` crossfades every free-running
//! instance of the root composition to the named animation (if it has one),
//! e.g. `--crossfade squash@1s/0.2s`.

use backstage_core::{DrawContent, RuntimeState, Time, evaluate, load};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || -> ! {
        eprintln!(
            "usage: scene_dump <project.bs2d> [time, e.g. 0.5s] [--crossfade <anim>@<start>/<duration>]"
        );
        std::process::exit(2);
    };
    let (mut positional, mut crossfade) = (Vec::new(), None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--crossfade" {
            crossfade = Some(it.next().unwrap_or_else(|| usage()).clone());
        } else {
            positional.push(arg.clone());
        }
    }
    let Some(dir) = positional.first() else { usage() };
    let project = load(&PathBuf::from(dir))?;
    let now: Time = positional.get(1).map_or("0s", String::as_str).parse()?;

    let mut state = RuntimeState::default();
    if let Some(spec) = crossfade {
        let (name, timing) = spec.split_once('@').unwrap_or_else(|| usage());
        let (start, duration) = timing.split_once('/').unwrap_or_else(|| usage());
        let (start, duration): (Time, Time) = (start.parse()?, duration.parse()?);
        let root = &project.compositions[&project.root];
        for (id, node) in &root.nodes {
            let backstage_core::NodeKind::Instance(i) = &node.kind else { continue };
            if !matches!(i.time, backstage_core::TimeMode::Free { .. }) {
                continue;
            }
            let comp = &project.compositions[&i.comp];
            if let Some((anim, _)) = comp.animations.iter().find(|(_, a)| a.name == name) {
                state.crossfade(&project, &vec![*id], *anim, start, duration);
                println!("crossfading {} ({}) to {name:?} at {start} over {duration}", node.name, comp.name);
            }
        }
    }
    let scene = evaluate(&project, &state, now);
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
            "  {:<28} {:<10} at ({:7.2}, {:7.2})  scale ({:.3}, {:.3})  opacity {:.2}  {}",
            format!("{}/{}", comp.name, comp.nodes[&item.node].name),
            format!("[{}]", path.len() - 1),
            pos.x,
            pos.y,
            item.transform.transform_vector2(backstage_core::Vec2::X).length(),
            item.transform.transform_vector2(backstage_core::Vec2::Y).length(),
            item.opacity,
            content
        );
    }
    Ok(())
}
