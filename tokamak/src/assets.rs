//! A Worker's static assets, served as Cloudflare's asset worker serves them.

use std::io;
use std::path::PathBuf;

use hyper::StatusCode;
use hyper::header::{CONTENT_TYPE, HeaderMap, HeaderValue};

use crate::packaging::{self, AssetManifest, HtmlHandling, NotFoundHandling, PackageLayout};
use crate::transport::{HttpRequest, HttpResponse};

/// A packaged app's static assets.
#[derive(Debug)]
pub(crate) struct Assets {
    root: PathBuf,
    manifest: AssetManifest,
}

/// What the assets answer a request with.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct AssetResponse {
    pub(crate) status: StatusCode,
    /// The served asset's content type.
    pub(crate) content_type: Option<String>,
    /// The served asset's contents, unless the request is `HEAD`.
    pub(crate) body: Option<Vec<u8>>,
}

impl Assets {
    /// The assets packaged in `app`.
    pub(crate) fn open(app: &PackageLayout) -> packaging::Result<Self> {
        Ok(Self::new(
            app.assets(),
            packaging::read_asset_manifest(app)?,
        ))
    }

    /// The assets in `root` that `manifest` describes.
    pub(crate) fn new(root: PathBuf, manifest: AssetManifest) -> Self {
        Self { root, manifest }
    }

    /// The name of the Worker's binding to the assets.
    pub(crate) fn binding(&self) -> &str {
        &self.manifest.binding
    }

    /// The answer to `request` when it goes to the assets rather than the
    /// Worker: when an asset serves its path or, for a navigation, when
    /// not-found handling answers it.
    pub(crate) fn route(&self, request: &HttpRequest) -> io::Result<Option<HttpResponse>> {
        let navigation = request
            .headers
            .get("sec-fetch-mode")
            .is_some_and(|mode| mode == "navigate");
        let not_found = if navigation {
            self.manifest.not_found_handling
        } else {
            NotFoundHandling::None
        };
        let path = request.target.split('?').next().unwrap_or("/");
        self.respond(&request.method, path, not_found)?
            .map(AssetResponse::into_http)
            .transpose()
    }

    /// The answer of the binding's `fetch` to a `method` request for `path`.
    pub(crate) fn fetch(&self, method: &str, path: &str) -> AssetResponse {
        match self.respond(method, path, self.manifest.not_found_handling) {
            Ok(Some(response)) => response,
            Ok(None) => AssetResponse::empty(StatusCode::NOT_FOUND),
            Err(_) => AssetResponse::empty(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }

    fn respond(
        &self,
        method: &str,
        path: &str,
        not_found: NotFoundHandling,
    ) -> io::Result<Option<AssetResponse>> {
        let Some((file, status)) = self.resolve(path, not_found, true) else {
            return Ok(None);
        };
        let head = method.eq_ignore_ascii_case("HEAD");
        if !head && !method.eq_ignore_ascii_case("GET") {
            return Ok(Some(AssetResponse::empty(StatusCode::METHOD_NOT_ALLOWED)));
        }
        let body = if head {
            None
        } else {
            Some(std::fs::read(self.root.join(file))?)
        };
        Ok(Some(AssetResponse {
            status,
            content_type: self.manifest.files.get(file).cloned(),
            body,
        }))
    }

    /// The asset serving `path`, and the status it is served with.
    fn resolve(
        &self,
        path: &str,
        not_found: NotFoundHandling,
        follow_redirects: bool,
    ) -> Option<(&str, StatusCode)> {
        for (asset, redirected) in html_steps(self.manifest.html_handling, path) {
            let Some(file) = self.file(&asset) else {
                continue;
            };
            if !redirected || follow_redirects && self.redirects_to(&asset, file, not_found) {
                return Some((file, StatusCode::OK));
            }
        }
        match not_found {
            NotFoundHandling::None => None,
            NotFoundHandling::SinglePageApplication => {
                Some((self.file("/index.html")?, StatusCode::OK))
            }
            NotFoundHandling::Page404 => {
                Some((self.nearest_404_page(path)?, StatusCode::NOT_FOUND))
            }
        }
    }

    /// Whether Cloudflare redirects to the path it serves `file`, at `asset`,
    /// at: when no asset is at that path and, without redirects, it serves
    /// `file`.
    fn redirects_to(&self, asset: &str, file: &str, not_found: NotFoundHandling) -> bool {
        let destination = html_path(self.manifest.html_handling, asset);
        self.file(&destination).is_none()
            && self
                .resolve(&destination, not_found, false)
                .is_some_and(|(served, _)| served == file)
    }

    /// The `404.html` in `path`'s directory or the closest one above it.
    fn nearest_404_page(&self, path: &str) -> Option<&str> {
        let mut directory = path;
        while let Some((parent, _)) = directory.rsplit_once('/') {
            if let Some(file) = self.file(&format!("{parent}/404.html")) {
                return Some(file);
            }
            directory = parent;
        }
        None
    }

    /// The asset at `path`.
    fn file(&self, path: &str) -> Option<&str> {
        let (file, _) = self.manifest.files.get_key_value(path.strip_prefix('/')?)?;
        Some(file)
    }
}

/// The path of an asset that may serve a request, and whether Cloudflare
/// redirects to the path it serves the asset at. tokamak serves the asset in
/// both cases.
type Step = (String, bool);

fn serve(asset: impl Into<String>) -> Step {
    (asset.into(), false)
}

fn redirect(asset: impl Into<String>) -> Step {
    (asset.into(), true)
}

/// The path Cloudflare serves the HTML asset at `path` at under `handling`.
fn html_path(handling: HtmlHandling, path: &str) -> String {
    let (page, index) = match path.strip_suffix("/index.html") {
        Some(page) => (page, true),
        None => (path.strip_suffix(".html").unwrap_or(path), false),
    };
    let trailing_slash = match handling {
        HtmlHandling::AutoTrailingSlash => index,
        HtmlHandling::ForceTrailingSlash => true,
        HtmlHandling::DropTrailingSlash | HtmlHandling::None => false,
    };
    if trailing_slash || page.is_empty() {
        format!("{page}/")
    } else {
        page.to_owned()
    }
}

/// The assets that may serve `path` under `handling`, in the order
/// Cloudflare's asset worker tries them.
fn html_steps(handling: HtmlHandling, path: &str) -> Vec<Step> {
    match handling {
        HtmlHandling::AutoTrailingSlash => auto_trailing_slash(path),
        HtmlHandling::ForceTrailingSlash => force_trailing_slash(path),
        HtmlHandling::DropTrailingSlash => drop_trailing_slash(path),
        HtmlHandling::None => vec![serve(path)],
    }
}

fn auto_trailing_slash(path: &str) -> Vec<Step> {
    let mut steps = if let Some(page) = path.strip_suffix("/index") {
        vec![
            serve(path),
            redirect(format!("{path}.html")),
            redirect(format!("{page}.html")),
        ]
    } else if let Some(page) = path.strip_suffix("/index.html") {
        vec![redirect(path), redirect(format!("{page}.html"))]
    } else if let Some(page) = path.strip_suffix('/') {
        vec![
            serve(format!("{path}index.html")),
            redirect(format!("{page}.html")),
        ]
    } else if let Some(page) = path.strip_suffix(".html") {
        vec![redirect(path), redirect(format!("{page}/index.html"))]
    } else {
        Vec::new()
    };
    steps.extend([
        serve(path),
        serve(format!("{path}.html")),
        redirect(format!("{path}/index.html")),
    ]);
    steps
}

fn force_trailing_slash(path: &str) -> Vec<Step> {
    let mut steps = if let Some(page) = path.strip_suffix("/index") {
        vec![
            serve(path),
            redirect(format!("{path}.html")),
            redirect(format!("{page}.html")),
        ]
    } else if let Some(page) = path.strip_suffix("/index.html") {
        vec![redirect(path), redirect(format!("{page}.html"))]
    } else if let Some(page) = path.strip_suffix('/') {
        vec![
            serve(format!("{path}index.html")),
            serve(format!("{page}.html")),
        ]
    } else if let Some(page) = path.strip_suffix(".html") {
        vec![
            redirect(path),
            serve(path),
            redirect(format!("{page}/index.html")),
        ]
    } else {
        Vec::new()
    };
    steps.extend([
        serve(path),
        redirect(format!("{path}.html")),
        redirect(format!("{path}/index.html")),
    ]);
    steps
}

fn drop_trailing_slash(path: &str) -> Vec<Step> {
    let mut steps = if path == "/" {
        vec![serve("/index.html")]
    } else if let Some(page) = path.strip_suffix("/index") {
        vec![
            serve(path),
            redirect(format!("{page}.html")),
            redirect(format!("{path}.html")),
        ]
    } else if let Some(page) = path.strip_suffix("/index.html") {
        vec![
            redirect(path),
            serve(path),
            redirect(format!("{page}.html")),
        ]
    } else if let Some(page) = path.strip_suffix('/') {
        vec![
            redirect(format!("{page}.html")),
            redirect(format!("{page}/index.html")),
        ]
    } else if let Some(page) = path.strip_suffix(".html") {
        vec![redirect(path), redirect(format!("{page}/index.html"))]
    } else {
        Vec::new()
    };
    steps.extend([
        serve(path),
        serve(format!("{path}.html")),
        serve(format!("{path}/index.html")),
    ]);
    steps
}

impl AssetResponse {
    fn empty(status: StatusCode) -> Self {
        Self {
            status,
            content_type: None,
            body: None,
        }
    }

    fn into_http(self) -> io::Result<HttpResponse> {
        let mut headers = HeaderMap::new();
        if let Some(content_type) = self.content_type {
            let content_type = HeaderValue::try_from(content_type).map_err(io::Error::other)?;
            headers.insert(CONTENT_TYPE, content_type);
        }
        Ok(HttpResponse::buffered(
            self.status.as_u16(),
            headers,
            self.body.unwrap_or_default(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use hyper::StatusCode;

    use super::{AssetResponse, Assets};
    use crate::packaging::{
        AssetManifest, HtmlHandling, NotFoundHandling, PackageLayout, write_asset_manifest,
    };

    #[test]
    fn serves_an_asset_with_its_content_type_while_its_file_exists()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path());
        fs::create_dir_all(app.assets())?;
        fs::write(app.assets().join("app.css"), "p {}")?;
        let manifest = AssetManifest {
            binding: "STATIC".to_owned(),
            files: BTreeMap::from([("app.css".to_owned(), "text/css".to_owned())]),
            html_handling: HtmlHandling::default(),
            not_found_handling: NotFoundHandling::default(),
        };
        write_asset_manifest(&app, &manifest)?;
        let assets = Assets::open(&app)?;

        let response = assets.fetch("GET", "/app.css");
        assert_eq!(
            response,
            AssetResponse {
                status: StatusCode::OK,
                content_type: Some("text/css".to_owned()),
                body: Some(b"p {}".to_vec()),
            }
        );
        assert_eq!(response.into_http()?.headers["content-type"], "text/css");
        fs::remove_file(app.assets().join("app.css"))?;
        assert_eq!(
            assets.fetch("GET", "/app.css").status,
            StatusCode::INTERNAL_SERVER_ERROR
        );
        Ok(())
    }
}
