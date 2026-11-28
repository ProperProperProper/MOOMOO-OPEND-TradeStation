export const VERSION = '0.1.1';

/** `scripts/release-macos.sh` builds the release DMG as a universal binary. */
export const ARCH = 'Apple Silicon and Intel';
export const MIN_MACOS = '11';

export const NAME = 'Terminal';
/** `static/robots.txt` and `static/sitemap.xml` repeat this origin. */
export const SITE_URL = 'https://terminal.rasmic.xyz';
export const REPO_URL = 'https://github.com/ProperProperProper/MOOMOO-OPEND-TradeStation';
export const DOWNLOAD_URL = `${REPO_URL}/releases/latest/download/Terminal.dmg`;

export const TITLE = `${NAME}: automated AI quantitative trading platform`;
export const DESCRIPTION =
	'An automated, strategy-driven, and AI-driven quantitative trading platform and autonomous execution engine for the Mac, written in Rust. GPU charts, DRBT-Grid strategies, and local Moomoo OpenD integration.';
export const OG_IMAGE_ALT = `${NAME}: the market, one command away. The app icon beside a screenshot of a watchlist, an AAPL chart and world markets, with AAPL typed into a command bar.`;
