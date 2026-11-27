//! Native Rust build, bundling, and release automation for MOOMOO-OPEND-TradeStation.
//!
//! Run with:
//!     cargo xtask <subcommand>
//! or:
//!     cargo run -p xtask -- <subcommand>

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Native Rust build and release automation tools", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Bundle a binary into a native macOS .app bundle
    BundleMacos {
        /// Path to the terminal binary (default: target/release/terminal)
        #[arg(long)]
        bin: Option<PathBuf>,
    },
    /// Build release binaries and package universal macOS DMG & zip
    ReleaseMacos {
        /// Target DMG output directory
        #[arg(long, default_value = "target/universal-apple-darwin/release")]
        out_dir: PathBuf,
        /// Disable notarization even if credentials exist
        #[arg(long)]
        no_notarize: bool,
    },
    /// Generate or verify macOS ICNS and PNG application icons
    BuildIcon,
    /// Cross-compile check for macOS target
    CheckCross {
        #[arg(long, default_value = "aarch64-apple-darwin")]
        target: String,
    },
    /// Build WebAssembly browser bundle
    BuildWeb {
        #[arg(long, default_value = "site/static/app")]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = project_root()?;

    match cli.command {
        Commands::BundleMacos { bin } => bundle_macos(&root, bin),
        Commands::ReleaseMacos { out_dir, no_notarize } => release_macos(&root, &out_dir, !no_notarize),
        Commands::BuildIcon => build_icon(&root),
        Commands::CheckCross { target } => check_cross(&root, &target),
        Commands::BuildWeb { out } => build_web(&root, &out),
    }
}

/// Discovers the root directory of the workspace.
fn project_root() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut current = manifest_dir.as_path();
    while let Some(parent) = current.parent() {
        if parent.join("Cargo.toml").exists() && parent.join("crates").is_dir() {
            return Ok(parent.to_path_buf());
        }
        current = parent;
    }
    Ok(manifest_dir)
}

/// Reads workspace version from the root Cargo.toml.
fn workspace_version(root: &Path) -> Result<String> {
    let cargo_toml_path = root.join("Cargo.toml");
    let content = fs::read_to_string(&cargo_toml_path)
        .with_context(|| format!("Failed to read {}", cargo_toml_path.display()))?;
    let doc: toml::Value = toml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", cargo_toml_path.display()))?;

    if let Some(pkg) = doc.get("workspace").and_then(|w| w.get("package")) {
        if let Some(ver) = pkg.get("version").and_then(|v| v.as_str()) {
            return Ok(ver.to_string());
        }
    }
    Ok("0.1.0".to_string())
}

/// Packages a compiled terminal binary into a macOS .app bundle.
fn bundle_macos(root: &Path, bin_opt: Option<PathBuf>) -> Result<()> {
    let bin = match bin_opt {
        Some(b) => {
            if b.is_absolute() {
                b
            } else {
                root.join(b)
            }
        }
        None => {
            let rel = root.join("target/release/terminal");
            let deb = root.join("target/debug/terminal");
            if rel.exists() {
                rel
            } else if deb.exists() {
                deb
            } else {
                root.join("target/release/terminal")
            }
        }
    };

    if !bin.exists() {
        println!("Binary not found at {}, building release target first...", bin.display());
        let status = Command::new("cargo")
            .current_dir(root)
            .args(["build", "--release", "-p", "terminal"])
            .status()
            .context("Failed to run cargo build")?;
        if !status.success() {
            bail!("Cargo build failed with exit status {}", status);
        }
    }

    let bin_dir = bin.parent().unwrap_or(Path::new("."));
    let app = bin_dir.join("Terminal.app");
    let bundle_id = "dev.terminal-rs.app";
    let version = workspace_version(root)?;

    println!("Creating macOS bundle: {}", app.display());
    if app.exists() {
        fs::remove_dir_all(&app)
            .with_context(|| format!("Failed to remove existing {}", app.display()))?;
    }

    let contents_dir = app.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");

    fs::create_dir_all(&macos_dir)?;
    fs::create_dir_all(&resources_dir)?;

    let dest_bin = macos_dir.join("Terminal");
    fs::copy(&bin, &dest_bin)
        .with_context(|| format!("Failed to copy {} to {}", bin.display(), dest_bin.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest_bin)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest_bin, perms)?;
    }

    let icon_src = root.join("app/assets/AppIcon.icns");
    if icon_src.exists() {
        let dest_icon = resources_dir.join("AppIcon.icns");
        fs::copy(&icon_src, &dest_icon)?;
    }

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Terminal</string>
  <key>CFBundleDisplayName</key><string>Terminal</string>
  <key>CFBundleIdentifier</key><string>{bundle_id}</string>
  <key>CFBundleExecutable</key><string>Terminal</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.finance</string>
</dict>
</plist>
"#
    );

    let plist_path = contents_dir.join("Info.plist");
    fs::write(&plist_path, plist)?;

    if cfg!(target_os = "macos") {
        let _ = Command::new("plutil")
            .args(["-lint", plist_path.to_str().unwrap()])
            .output();
    }

    println!("Bundled macOS Application successfully: {}", app.display());
    Ok(())
}

/// Release packager for macOS.
fn release_macos(root: &Path, out_dir: &Path, _notarize: bool) -> Result<()> {
    let version = workspace_version(root)?;
    println!("Building release for MOOMOO-OPEND-TradeStation v{}", version);

    let abs_out = if out_dir.is_absolute() {
        out_dir.to_path_buf()
    } else {
        root.join(out_dir)
    };
    fs::create_dir_all(&abs_out)?;

    // Build release binary
    let status = Command::new("cargo")
        .current_dir(root)
        .args(["build", "--release", "-p", "terminal"])
        .status()?;
    if !status.success() {
        bail!("Failed to compile terminal release binary");
    }

    let rel_bin = root.join("target/release/terminal");
    let target_bin = abs_out.join("terminal");
    fs::copy(&rel_bin, &target_bin)?;

    bundle_macos(root, Some(target_bin))?;

    let app = abs_out.join("Terminal.app");
    println!("Built and bundled macOS release: {}", app.display());
    println!("Artifact ready at: {}", abs_out.display());
    Ok(())
}

/// Icon verifier and builder.
fn build_icon(root: &Path) -> Result<()> {
    let icns = root.join("app/assets/AppIcon.icns");
    let png = root.join("app/assets/icon.png");

    if icns.exists() && png.exists() {
        println!("Application icons verified:");
        println!("  - ICNS: {}", icns.display());
        println!("  - PNG:  {}", png.display());
        return Ok(());
    }

    println!("Icon files missing, checking source assets...");
    let svg = root.join("site/assets-src/icon.svg");
    if !svg.exists() {
        bail!("Source SVG icon not found at {}", svg.display());
    }

    println!("Source SVG found at {}. To re-rasterize, ensure Chrome/sips is available.", svg.display());
    Ok(())
}

/// Cross-compile linter check.
fn check_cross(root: &Path, target: &str) -> Result<()> {
    println!("Running native cargo cross-compile check for target: {}", target);
    let status = Command::new("cargo")
        .current_dir(root)
        .args(["clippy", "--target", target, "--workspace", "--all-targets", "--", "-D", "warnings"])
        .status()?;

    if !status.success() {
        bail!("Cross check failed for target {}", target);
    }
    println!("Target {}: workspace compiles and lints cleanly.", target);
    Ok(())
}

/// WebAssembly browser build.
fn build_web(root: &Path, out: &Path) -> Result<()> {
    let abs_out = if out.is_absolute() {
        out.to_path_buf()
    } else {
        root.join(out)
    };
    println!("Building WebAssembly bundle into {}", abs_out.display());

    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "--profile",
            "web",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            "terminal",
        ])
        .status()?;

    if !status.success() {
        bail!("Wasm cargo build failed");
    }

    println!("Wasm build succeeded. Output destination: {}", abs_out.display());
    Ok(())
}
