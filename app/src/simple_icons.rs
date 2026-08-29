//! Embedded Simple Icons catalog and GPUI SVG asset source.
//!
//! Brand icons are stored in profile JSON as `simple:<slug>`. The generated
//! catalog contains only each icon's slug, human-readable title, and SVG path;
//! [`Assets`] adds the common SVG wrapper on demand so release
//! packages do not need thousands of loose files.

use gpui::{AssetSource, Result, SharedString};
use serde::Deserialize;
use std::{borrow::Cow, sync::LazyLock};

/// Namespace used for Simple Icons values in `InputConfig.icon`.
pub const STORAGE_PREFIX: &str = "simple:";

/// Virtual directory served by [`Assets`].
pub const ASSET_DIRECTORY: &str = "simple-icons";

const CATALOG_JSON: &str = include_str!("../resources/simple-icons.json");
const CLEAR_ICON_PATH: &str = "icons/circle-x.svg";
const CLEAR_ICON_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="2" stroke-linecap="round"><path d="M6 6l12 12M18 6 6 18"/></svg>"##;
const WINDOW_CONTROL_ASSETS: &[(&str, &[u8])] = &[
    (
        "icons/window-minimize.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="#000" d="M3 8h10v1H3z"/></svg>"##,
    ),
    (
        "icons/window-maximize.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="#000" fill-rule="evenodd" d="M3 3h10v10H3V3zm1 1v8h8V4H4z"/></svg>"##,
    ),
    (
        "icons/window-restore.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="#000" fill-rule="evenodd" d="M5 3h8v8h-2v2H3V5h2V3zm1 1v1h5v5h1V4H6zM4 6v6h6V6H4z"/></svg>"##,
    ),
    (
        "icons/window-close.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill="#000" d="M3.6 2.9 8 7.3l4.4-4.4.7.7L8.7 8l4.4 4.4-.7.7L8 8.7l-4.4 4.4-.7-.7L7.3 8 2.9 3.6z"/></svg>"##,
    ),
];

/// One bundled monochrome brand icon.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimpleIcon {
    pub slug: String,
    pub title: String,
    pub path: String,
}

#[derive(Deserialize)]
struct CatalogFile {
    version: String,
    icons: Vec<(String, String, String)>,
}

struct Catalog {
    version: String,
    icons: Vec<SimpleIcon>,
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    let raw: CatalogFile =
        serde_json::from_str(CATALOG_JSON).expect("embedded Simple Icons catalog must be valid");
    let icons = raw
        .icons
        .into_iter()
        .map(|(slug, title, path)| SimpleIcon { slug, title, path })
        .collect::<Vec<_>>();
    assert!(
        icons.windows(2).all(|pair| pair[0].slug < pair[1].slug),
        "embedded Simple Icons catalog must be sorted by unique slug"
    );
    Catalog {
        version: raw.version,
        icons,
    }
});

/// Version of the Simple Icons npm package used to generate this catalog.
pub fn version() -> &'static str {
    &CATALOG.version
}

/// All bundled icons, sorted by slug.
pub fn icons() -> &'static [SimpleIcon] {
    &CATALOG.icons
}

/// Find a bundled icon by its exact lowercase Simple Icons slug.
pub fn find(slug: &str) -> Option<&'static SimpleIcon> {
    icons()
        .binary_search_by(|icon| icon.slug.as_str().cmp(slug))
        .ok()
        .map(|index| &icons()[index])
}

/// Search titles and slugs case-insensitively.
///
/// An empty or whitespace-only query returns the full catalog in slug order.
pub fn search(query: &str) -> Vec<&'static SimpleIcon> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return icons().iter().collect();
    }
    icons()
        .iter()
        .filter(|icon| {
            icon.slug.contains(&query) || icon.title.to_lowercase().contains(query.as_str())
        })
        .collect()
}

/// Parse the slug from a namespaced persisted value.
///
/// This recognizes canonical Simple Icons syntax without requiring that the
/// slug exists in this particular catalog version. Call [`find`] when the
/// caller also needs to validate availability.
pub fn slug_from_storage(value: &str) -> Option<&str> {
    let slug = value.strip_prefix(STORAGE_PREFIX)?;
    is_valid_slug(slug).then_some(slug)
}

/// Build a canonical persisted value for a Simple Icons slug.
pub fn storage_value(slug: &str) -> String {
    format!("{STORAGE_PREFIX}{slug}")
}

/// Return the GPUI virtual asset path for a Simple Icons slug.
pub fn asset_path(slug: &str) -> SharedString {
    SharedString::from(format!("{ASSET_DIRECTORY}/{slug}.svg"))
}

fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn slug_from_asset_path(path: &str) -> Option<&str> {
    let file = path.strip_prefix(ASSET_DIRECTORY)?.strip_prefix('/')?;
    let slug = file.strip_suffix(".svg")?;
    is_valid_slug(slug).then_some(slug)
}

fn svg_document(icon: &SimpleIcon) -> Vec<u8> {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000" d="{}"/></svg>"##,
        icon.path
    )
    .into_bytes()
}

/// Serves the embedded catalog as monochrome `simple-icons/<slug>.svg` assets.
///
/// Install it with `Application::new().with_assets(Assets)` before
/// rendering GPUI `svg().path(...)` elements.
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == CLEAR_ICON_PATH {
            return Ok(Some(Cow::Borrowed(CLEAR_ICON_SVG)));
        }
        if let Some((_, contents)) = WINDOW_CONTROL_ASSETS
            .iter()
            .find(|(asset_path, _)| *asset_path == path)
        {
            return Ok(Some(Cow::Borrowed(contents)));
        }
        let Some(slug) = slug_from_asset_path(path) else {
            return Ok(None);
        };
        Ok(find(slug).map(|icon| Cow::Owned(svg_document(icon))))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        match path.trim_end_matches('/') {
            "" => Ok(vec![
                SharedString::from(ASSET_DIRECTORY),
                SharedString::from("icons"),
            ]),
            "icons" => Ok(std::iter::once(SharedString::from("circle-x.svg"))
                .chain(WINDOW_CONTROL_ASSETS.iter().map(|(path, _)| {
                    SharedString::from(path.trim_start_matches("icons/"))
                }))
                .collect()),
            ASSET_DIRECTORY => Ok(icons()
                .iter()
                .map(|icon| SharedString::from(format!("{}.svg", icon.slug)))
                .collect()),
            _ => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_complete_sorted_and_unique() {
        assert_eq!(version(), "16.27.1");
        assert_eq!(icons().len(), 3_450);
        assert!(icons().windows(2).all(|pair| pair[0].slug < pair[1].slug));
    }

    #[test]
    fn lookup_returns_brand_metadata_and_path() {
        let github = find("github").expect("GitHub must be bundled");
        assert_eq!(github.title, "GitHub");
        assert!(github.path.starts_with("M12 .297"));
        assert!(find("GitHub").is_none());
        assert!(find("not-a-real-brand").is_none());
    }

    #[test]
    fn search_matches_title_and_slug_case_insensitively() {
        assert!(search("GITHUB").iter().any(|icon| icon.slug == "github"));
        assert!(search("google cloud")
            .iter()
            .any(|icon| icon.slug == "googlecloud"));
        assert_eq!(search("  ").len(), icons().len());
    }

    #[test]
    fn storage_values_are_namespaced_and_backward_compatible() {
        assert_eq!(slug_from_storage("simple:github"), Some("github"));
        assert_eq!(storage_value("github"), "simple:github");
        assert_eq!(slug_from_storage("github"), None);
        assert_eq!(slug_from_storage("simple:"), None);
        assert_eq!(slug_from_storage("simple:../github"), None);
        assert_eq!(
            slug_from_storage("simple:unknown_brand"),
            Some("unknown_brand")
        );
    }

    #[test]
    fn asset_paths_are_strict_and_monochrome() {
        let source = Assets;
        let path = asset_path("github");
        assert_eq!(path.as_ref(), "simple-icons/github.svg");

        let bytes = source
            .load(path.as_ref())
            .expect("asset load must succeed")
            .expect("GitHub asset must exist");
        let svg = std::str::from_utf8(bytes.as_ref()).expect("SVG must be UTF-8");
        assert!(svg.contains(r#"viewBox="0 0 24 24""#));
        assert!(svg.contains(r##"fill="#000""##));
        assert!(svg.contains(&find("github").unwrap().path));

        assert!(source.load("simple-icons/unknown.svg").unwrap().is_none());
        assert!(source.load("simple-icons/../github.svg").unwrap().is_none());
        assert!(source.load("other/github.svg").unwrap().is_none());

        let clear = source
            .load(CLEAR_ICON_PATH)
            .unwrap()
            .expect("input clear icon must be served");
        assert!(std::str::from_utf8(clear.as_ref())
            .unwrap()
            .contains("M6 6l12 12"));
    }

    #[test]
    fn titlebar_control_assets_are_embedded() {
        let source = Assets;
        for (path, _) in WINDOW_CONTROL_ASSETS {
            let bytes = source
                .load(path)
                .expect("asset load")
                .expect("window control asset");
            assert!(bytes.starts_with(b"<svg"), "{path}");
        }
    }

    #[test]
    fn asset_listing_exposes_the_virtual_directory() {
        let source = Assets;
        assert_eq!(
            source.list("").unwrap(),
            vec![
                SharedString::from(ASSET_DIRECTORY),
                SharedString::from("icons")
            ]
        );
        let listed = source.list("simple-icons/").unwrap();
        assert_eq!(listed.len(), icons().len());
        assert!(listed.iter().any(|path| path.as_ref() == "github.svg"));
        let controls = source.list("icons").unwrap();
        assert_eq!(controls.len(), 1 + WINDOW_CONTROL_ASSETS.len());
        assert!(controls.iter().any(|path| path.as_ref() == "circle-x.svg"));
        assert!(controls
            .iter()
            .any(|path| path.as_ref() == "window-close.svg"));
        assert!(source.list("missing").unwrap().is_empty());
    }
}
