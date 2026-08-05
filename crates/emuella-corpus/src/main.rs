use emuella_corpus::{Catalogue, GENERATED_CORE_ID, generate_pack};
use std::env;
use std::error::Error;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.is_empty() || matches!(arguments[0].as_str(), "help" | "--help" | "-h") {
        print_help();
        return Ok(());
    }

    let root = find_catalogue_root()?;
    let catalogue = Catalogue::open(&root)?;
    match arguments.remove(0).as_str() {
        "list" => list(&catalogue, &arguments),
        "show" => show(&catalogue, &arguments),
        "check" => check(&catalogue, &arguments),
        "verify" => verify(&catalogue, &arguments),
        "inventory" => inventory(&catalogue, &arguments),
        "generate" => generate(&catalogue, &arguments),
        unknown => Err(format!("unknown command {unknown:?}; use --help").into()),
    }
}

fn inventory(catalogue: &Catalogue, arguments: &[String]) -> Result<(), Box<dyn Error>> {
    if arguments.len() != 5 || arguments[1] != "--root" || arguments[3] != "--output" {
        return Err("usage: emuella-corpus inventory <id> --root PATH --output PATH".into());
    }
    let report = catalogue.write_inventory(
        &arguments[0],
        &PathBuf::from(&arguments[2]),
        &PathBuf::from(&arguments[4]),
    )?;
    println!(
        "inventoried {} assets ({} bytes) with tree SHA-256 {} into {}",
        report.asset_count,
        report.total_bytes,
        report.tree_sha256,
        report.output.display()
    );
    Ok(())
}

fn list(catalogue: &Catalogue, arguments: &[String]) -> Result<(), Box<dyn Error>> {
    expect_arity("list", arguments, 1)?;
    match arguments[0].as_str() {
        "packs" => {
            for id in catalogue.pack_ids() {
                let pack = catalogue.pack(id).expect("ID came from catalogue");
                println!(
                    "{:<36} {:<24} {:<10?} {}",
                    id, pack.version, pack.review_state, pack.name
                );
            }
        }
        "suites" => {
            for id in catalogue.suite_ids() {
                let suite = catalogue.suite(id).expect("ID came from catalogue");
                println!(
                    "{:<36} r{:<4} layer {}  {}",
                    id, suite.revision, suite.layer, suite.name
                );
            }
        }
        unknown => {
            return Err(format!("unknown list kind {unknown:?}; use packs or suites").into());
        }
    }
    Ok(())
}

fn show(catalogue: &Catalogue, arguments: &[String]) -> Result<(), Box<dyn Error>> {
    expect_arity("show", arguments, 1)?;
    let id = &arguments[0];
    if let Some(pack) = catalogue.pack(id) {
        print!("{}", toml::to_string_pretty(pack)?);
    } else if let Some(suite) = catalogue.suite(id) {
        print!("{}", toml::to_string_pretty(suite)?);
    } else {
        return Err(format!("unknown pack or suite ID {id}").into());
    }
    Ok(())
}

fn check(catalogue: &Catalogue, arguments: &[String]) -> Result<(), Box<dyn Error>> {
    expect_arity("check", arguments, 0)?;
    let report = catalogue.check()?;
    println!(
        "checked {} packs, {} suites, and {} locked asset records",
        report.pack_count, report.suite_count, report.asset_count
    );
    for warning in report.warnings {
        println!("warning: {warning}");
    }
    Ok(())
}

fn verify(catalogue: &Catalogue, arguments: &[String]) -> Result<(), Box<dyn Error>> {
    if arguments.is_empty() {
        return Err("verify requires a pack ID".into());
    }
    let id = &arguments[0];
    let explicit_root = parse_output_option("verify", &arguments[1..], "--root")?;
    let report = catalogue.verify(id, explicit_root.as_deref())?;
    println!(
        "verified {} assets ({} bytes) for {} under {}",
        report.checked_assets,
        report.checked_bytes,
        report.pack_id,
        report.root.display()
    );
    if let Some(tree_sha256) = report.tree_sha256 {
        println!("verified complete tree SHA-256 {tree_sha256}");
    }
    Ok(())
}

fn generate(catalogue: &Catalogue, arguments: &[String]) -> Result<(), Box<dyn Error>> {
    if arguments.is_empty() {
        return Err("generate requires a pack ID".into());
    }
    let id = &arguments[0];
    let explicit_output = parse_output_option("generate", &arguments[1..], "--output")?;
    let output = explicit_output.unwrap_or(catalogue.default_materialization_root(id)?);
    let written = generate_pack(id, &output)?;
    println!(
        "generated {} files for {} under {}",
        written.len(),
        id,
        output.display()
    );
    Ok(())
}

fn parse_output_option(
    command: &str,
    arguments: &[String],
    option: &str,
) -> Result<Option<PathBuf>, Box<dyn Error>> {
    if arguments.is_empty() {
        return Ok(None);
    }
    if arguments.len() != 2 || arguments[0] != option {
        return Err(format!("usage: emuella-corpus {command} <id> [{option} PATH]").into());
    }
    Ok(Some(PathBuf::from(&arguments[1])))
}

fn expect_arity(
    command: &str,
    arguments: &[String],
    expected: usize,
) -> Result<(), Box<dyn Error>> {
    if arguments.len() != expected {
        return Err(format!(
            "command {command} expects {expected} argument(s), received {}",
            arguments.len()
        )
        .into());
    }
    Ok(())
}

fn find_catalogue_root() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(root) = env::var_os("EMU_TESTDATA_ROOT") {
        let root = PathBuf::from(root);
        if !root.join("catalog.toml").is_file() {
            return Err(format!(
                "EMU_TESTDATA_ROOT does not contain catalog.toml: {}",
                root.display()
            )
            .into());
        }
        return Ok(root);
    }

    let current = env::current_dir()?;
    for candidate in current.ancestors() {
        if candidate.join("catalog.toml").is_file() {
            return Ok(candidate.to_path_buf());
        }
    }
    Err(format!(
        "could not find catalog.toml from {}; set EMU_TESTDATA_ROOT",
        current.display()
    )
    .into())
}

fn print_help() {
    println!(
        "\
Emuella corpus catalogue

Usage:
  emuella-corpus list packs
  emuella-corpus list suites
  emuella-corpus show <pack-or-suite-id>
  emuella-corpus check
  emuella-corpus verify <pack-id> [--root PATH]
  emuella-corpus inventory <pack-id> --root PATH --output PATH
  emuella-corpus generate {GENERATED_CORE_ID} [--output PATH]

The catalogue root is found from EMU_TESTDATA_ROOT or by walking upward from
the current directory. External licence terms are never accepted automatically."
    );
}
