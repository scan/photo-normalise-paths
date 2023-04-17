use chrono::{DateTime, Datelike, Month, Utc};
use clap::Parser;
use futures::{stream, StreamExt};
use num_traits::cast::FromPrimitive;
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone, Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, value_hint = clap::ValueHint::DirPath)]
    pub source_dir: PathBuf,
    #[arg(short, long, value_hint = clap::ValueHint::DirPath)]
    pub dest_dir: PathBuf,
    #[arg(short, long, default_value_t = false)]
    pub recursive: bool,
    #[arg(short, long, value_parser, num_args = 0.., value_delimiter = ',', default_values_t = vec![
        "avif".to_owned(),
        "hif".to_owned(),
        "jpeg".to_owned(),
        "jpg".to_owned(),
        "png".to_owned(),
        "tif".to_owned(),
        "tiff".to_owned(),
        "dng".to_owned(),
        "arw".to_owned(),
        "raf".to_owned(),
    ])]
    pub file_extensions: Vec<String>,
}

async fn process_file(
    destination_path: impl AsRef<Path>,
    original_path: PathBuf,
) -> anyhow::Result<()> {
    let original_path = PathBuf::from(original_path.as_path());

    let file_attributes = fs::metadata(&original_path).await?;
    let creation_time: DateTime<Utc> = file_attributes.created()?.into();

    log::debug!("start processing file {}", original_path.display());

    let extension = original_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let new_file_name = original_path.with_extension(extension);

    let year = creation_time.year();
    let month = creation_time.month();
    let month_name = Month::from_u32(month);
    let day = creation_time.day();

    let final_dir = destination_path
        .as_ref()
        .join(year.to_string())
        .join(format!(
            "{:0>2} - {}",
            month,
            month_name.map_or("Unknown", |m| m.name())
        ))
        .join(format!("{:0>4}-{:0>2}-{:0>2}", year, month, day));
    let final_path = final_dir.join(new_file_name.file_name().unwrap_or_default());

    log::debug!("determined target path: {}", final_path.display());

    if !final_dir.is_dir() {
        fs::create_dir_all(&final_dir).await?;
    }

    log::info!(
        "moving file {} to {}",
        original_path.display(),
        final_path.display()
    );
    fs::rename(original_path, final_path).await?;

    for ext in vec!["dop", "xmp"] {
        let fpath = PathBuf::from(format!("{}.{}", new_file_name.display(), ext));

        if fs::try_exists(&fpath).await? {
            let npath = final_dir.join(fpath.file_name().unwrap_or_default());

            log::info!(
                "also moving metadata file {} to {}",
                fpath.display(),
                npath.display()
            );

            fs::rename(&fpath, &npath).await?;
        }
    }

    Ok(())
}

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let args = Args::parse();
    if !args.source_dir.is_dir() {
        anyhow::bail!(
            "The source path '{}' is not a directory",
            args.source_dir.display()
        );
    }
    if !args.dest_dir.is_dir() {
        anyhow::bail!(
            "The destination path '{}' is not a directory",
            args.dest_dir.display()
        );
    }

    let file_pattern = format!(
        "*.{{{}}}",
        args.file_extensions
            .iter()
            .map(|s| s.to_lowercase())
            .collect::<Vec<String>>()
            .join(",")
    );

    let file_paths: Vec<PathBuf> =
        globwalk::GlobWalkerBuilder::from_patterns(args.source_dir, &[file_pattern])
            .follow_links(false)
            .max_depth(if args.recursive { 5 } else { 1 })
            .case_insensitive(true)
            .build()
            .map_err(|e| anyhow::format_err!("{}", e.to_string()))?
            .filter_map(Result::ok)
            .map(|img| img.into_path())
            .collect();
    let dest_dir = args.dest_dir;

    stream::iter(file_paths)
        .map(|path| process_file(&dest_dir, path))
        .buffer_unordered(6)
        .for_each(|res| async {
            if let Err(e) = res {
                log::error!("failed to move file: {}", e);
            }
        })
        .await;

    Ok(())
}
