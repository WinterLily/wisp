use anyhow::{Context, Result};
use clap::Parser;
use std::{fs::OpenOptions, io::Write, path::PathBuf};
use wisp_compiler::{CompileOptions, compile_project};

#[derive(Parser)]
#[command(
    version,
    about = "Compile an editable Wisp graph project to a portable wallpaper"
)]
struct Args {
    /// Directory containing project.json, graph.json, and assets/.
    project: PathBuf,
    /// New .wisp output file. Existing files are never overwritten.
    #[arg(
        short,
        long,
        required_unless_present = "check",
        conflicts_with = "check"
    )]
    output: Option<PathBuf>,
    /// Compile and validate entirely on the CPU without writing a package.
    #[arg(long)]
    check: bool,
}
fn main() -> Result<()> {
    let args = Args::parse();
    let options = CompileOptions::default();
    let compilation = compile_project(&args.project, &options)?;
    if let Some(path) = args.output {
        let bytes = compilation.package.to_bytes(&options.package_limits)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "could not create {} (destination must be new)",
                    path.display()
                )
            })?;
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(error).context("could not write compiled package");
        }
        println!("Wrote {}", path.display());
    }
    let report = compilation.report;
    if report.resized_images > 0 {
        println!(
            "{} image(s) resized for playback; full-resolution project sources preserved.",
            report.resized_images
        );
    }
    println!(
        "Compiled: {} live nodes, {} dead nodes removed, {} passes, {} intermediate targets ({} bytes), {} image assets, {} parameters",
        report.live_nodes,
        report.eliminated_nodes,
        report.passes,
        report.intermediate_targets,
        report.intermediate_bytes,
        report.image_assets,
        report.parameters
    );
    Ok(())
}
