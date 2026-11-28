use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use crate::model::{StrategyParams, SymbolPaperConfig, TradingSystemConfig};

pub fn symbol_slug(symbol: &str) -> String {
    let s = symbol.trim().to_uppercase();
    let s = if !s.contains('.') && !s.contains('-') {
        format!("US.{}", s)
    } else {
        s
    };
    s.replace('.', "-")
}

pub fn slug_to_symbol(slug: &str) -> String {
    slug.replace('-', ".")
}

pub fn load_trading_system(root: &Path) -> Result<TradingSystemConfig> {
    let path = root.join("trading_system.json");
    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: TradingSystemConfig = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(config)
    } else {
        Ok(TradingSystemConfig::default())
    }
}

pub fn save_trading_system(root: &Path, config: &TradingSystemConfig) -> Result<()> {
    let path = root.join("trading_system.json");
    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, content)?;
    Ok(())
}

pub fn strategy_path(root: &Path, symbol: &str) -> PathBuf {
    let slug = symbol_slug(symbol);
    root.join("strategies").join(slug).join("paper.json")
}

pub fn load_symbol_strategy(root: &Path, symbol: &str) -> Result<SymbolPaperConfig> {
    let path = strategy_path(root, symbol);
    if path.exists() {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let config: SymbolPaperConfig = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(config)
    } else {
        Ok(SymbolPaperConfig {
            symbol: symbol.to_string(),
            interval: 10,
            mode: "SIMULATE".to_string(),
            params: StrategyParams::default(),
            run_id: "DEFAULT".to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        })
    }
}

pub fn save_symbol_strategy(root: &Path, config: &SymbolPaperConfig) -> Result<()> {
    let path = strategy_path(root, &config.symbol);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, content)?;
    Ok(())
}

pub fn list_configured_symbols(root: &Path) -> Vec<String> {
    let dir = root.join("strategies");
    let mut symbols = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str() {
                    symbols.push(slug_to_symbol(name));
                }
        }
    }
    symbols.sort();
    symbols
}
