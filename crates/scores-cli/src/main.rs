use clap::{Parser, Subcommand};
use scores_core::{Format, FormatRegistry};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "scores",
    version,
    about = "Convert between music score formats"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert a score file from one format to another.
    ///
    /// The format is inferred from each file's extension unless --from/--to is given.
    Convert {
        input: PathBuf,
        output: PathBuf,
        /// Input format id (see `scores list-formats`); inferred from the input's extension if omitted.
        #[arg(long)]
        from: Option<String>,
        /// Output format id (see `scores list-formats`); inferred from the output's extension if omitted.
        #[arg(long)]
        to: Option<String>,
    },
    /// List the score formats compiled into this binary.
    ListFormats,
}

fn build_registry() -> FormatRegistry {
    let mut registry = FormatRegistry::new();
    registry.register(Box::new(scores_format_musicxml::MusicXmlFormat));
    registry.register(Box::new(scores_format_mei::MeiFormat));
    registry.register(Box::new(scores_format_midi::MidiFormat));
    registry.register(Box::new(scores_format_abc::AbcFormat));
    registry.register(Box::new(scores_format_rtttl::RtttlFormat));
    registry.register(Box::new(scores_format_chordpro::ChordProFormat));
    registry.register(Box::new(scores_format_guitarpro::GuitarProFormat));
    registry.register(Box::new(scores_format_musescore::MuseScoreFormat));
    registry.register(Box::new(scores_format_primus::PrimusFormat));
    registry
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let registry = build_registry();

    match run(cli.command, &registry) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command, registry: &FormatRegistry) -> Result<(), String> {
    match command {
        Command::ListFormats => {
            for format in registry.iter() {
                println!(
                    "{:<10} {:<16} extensions: {}",
                    format.id(),
                    format.name(),
                    format.extensions().join(", ")
                );
            }
            Ok(())
        }
        Command::Convert {
            input,
            output,
            from,
            to,
        } => {
            let from_format = resolve_format(registry, from.as_deref(), &input, "input")?;
            let to_format = resolve_format(registry, to.as_deref(), &output, "output")?;

            let data = std::fs::read(&input)
                .map_err(|e| format!("failed to read {}: {e}", input.display()))?;
            let score = from_format.read(&data).map_err(|e| {
                format!(
                    "failed to parse {} as {}: {e}",
                    input.display(),
                    from_format.id()
                )
            })?;
            let bytes = to_format
                .write(&score)
                .map_err(|e| format!("failed to convert to {}: {e}", to_format.id()))?;
            std::fs::write(&output, bytes)
                .map_err(|e| format!("failed to write {}: {e}", output.display()))?;

            println!(
                "Converted {} ({}) -> {} ({})",
                input.display(),
                from_format.id(),
                output.display(),
                to_format.id()
            );
            Ok(())
        }
    }
}

fn resolve_format<'a>(
    registry: &'a FormatRegistry,
    explicit: Option<&str>,
    path: &Path,
    role: &str,
) -> Result<&'a dyn Format, String> {
    if let Some(id) = explicit {
        return registry
            .by_id(id)
            .ok_or_else(|| format!("unknown format '{id}' (see `scores list-formats`)"));
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| format!("cannot infer a format for {} ({role} has no file extension) - pass --from/--to explicitly", path.display()))?;
    registry.by_extension(extension).ok_or_else(|| {
        format!("no registered format for extension '.{extension}' - pass --from/--to explicitly")
    })
}
