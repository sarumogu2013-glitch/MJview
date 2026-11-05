//! 自動アップデート（設計書 9章）: GitHub Releases の最新版を確かめ、zip を取って、実行ファイルの隣に入れ替える。
//!
//! - 公開リポジトリなのでトークンは使わない
//! - 本番配信中に勝手に入れ替えないよう、「確認」「取得」「適用」を別々の関数にしている。
//!   アプリは起動時に確認だけを行い、適用は人がボタンを押したときだけ行う
//! - 配布 zip の名前は `janscope-<版>-<対象>.zip`、隣に SHA-256 を書いた `<zip名>.sha256` を置く（リリースの作業で作る）
//! - 適用は「今のファイルを `.old` に改名 → 新しいファイルを置く」の順。Windows でも動いている exe は改名ならできる。
//!   `.old` は次の起動時に [`cleanup_old`] で消す
//! - 利用者のファイル（設定・データ・ffmpeg）は zip に入れないので、入れ替えで消えることはない

use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub use semver;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// 既定の取得元
pub const DEFAULT_REPO: &str = "sarumogu2013-glitch/MJviwe";
pub const GITHUB_API: &str = "https://api.github.com";
/// 配布物のフォルダに置く印。これがないフォルダ（開発中の target など）には適用しない
pub const DIST_MARKER: &str = "VERSION.txt";
/// 作業用フォルダ（配布フォルダの中に作る。改名が同じドライブで済むように）
pub const WORK_DIR: &str = "update-work";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("通信できません: {0}")]
    Http(String),
    #[error("サーバーがエラーを返しました（HTTP {0}）")]
    Status(u16),
    #[error("公開されているリリースがありません（まだ出していないか、リポジトリが非公開）")]
    NoRelease,
    #[error("リリースの情報が読めません: {0}")]
    Parse(String),
    #[error("このPC向けのファイルがリリースにありません（{0}）")]
    NoAsset(String),
    #[error("ダウンロードしたファイルが壊れています（SHA-256 が一致しません）")]
    Checksum,
    #[error("zip を展開できません: {0}")]
    Zip(String),
    #[error("配布フォルダではないので適用しません（{0} がありません）")]
    NotDist(String),
    #[error("ファイルの操作に失敗しました: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// この実行ファイル向けの配布物の名前の末尾
pub fn platform_suffix() -> &'static str {
    if cfg!(windows) {
        "windows-x64.zip"
    } else if cfg!(target_os = "macos") {
        "macos.zip"
    } else {
        "linux-x64.zip"
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: semver::Version,
    /// リリースの説明文（変更点）
    pub notes: String,
    pub page_url: String,
    pub zip: Option<Asset>,
    pub checksum: Option<Asset>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// 最新（または手元のほうが新しい）
    UpToDate { latest: semver::Version },
    /// 新しい版がある
    Available(Release),
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// タグ（`v1.2.3` / `1.2.3`）を版に直す
pub fn parse_tag(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim().trim_start_matches(['v', 'V'])).ok()
}

fn to_asset(a: &GhAsset) -> Asset {
    Asset {
        name: a.name.clone(),
        url: a.browser_download_url.clone(),
        size: a.size,
    }
}

/// GitHub API の release の JSON を読む（下書き・プレリリースは None）
pub fn parse_release(json: &str, suffix: &str) -> Result<Option<Release>> {
    let r: GhRelease = serde_json::from_str(json).map_err(|e| Error::Parse(e.to_string()))?;
    if r.draft || r.prerelease {
        return Ok(None);
    }
    let version = parse_tag(&r.tag_name)
        .ok_or_else(|| Error::Parse(format!("タグ {} が版の形ではありません", r.tag_name)))?;
    let zip = r
        .assets
        .iter()
        .find(|a| a.name.starts_with("janscope-") && a.name.ends_with(suffix))
        .map(to_asset);
    let checksum = zip.as_ref().and_then(|z| {
        let want = format!("{}.sha256", z.name);
        r.assets.iter().find(|a| a.name == want).map(to_asset)
    });
    Ok(Some(Release {
        tag: r.tag_name,
        version,
        notes: r.body.unwrap_or_default(),
        page_url: r.html_url,
        zip,
        checksum,
    }))
}

/// 取得元とHTTPの設定
pub struct Updater {
    agent: ureq::Agent,
    api_base: String,
    repo: String,
    suffix: String,
}

impl Updater {
    /// GitHub の既定のリポジトリから取る（プロキシは環境変数 HTTPS_PROXY / NO_PROXY に従う）。
    /// 確認用に、環境変数 JANSCOPE_UPDATE_API で取得元の根元を変えられる
    pub fn github() -> Self {
        let api = std::env::var("JANSCOPE_UPDATE_API").unwrap_or_else(|_| GITHUB_API.into());
        Self::new(&api, DEFAULT_REPO, true)
    }

    /// `api_base` は `https://api.github.com` のような根元。`use_proxy` が false なら環境変数のプロキシを使わない（テスト用）
    pub fn new(api_base: &str, repo: &str, use_proxy: bool) -> Self {
        let mut b = ureq::Agent::config_builder()
            .user_agent(format!("janscope-updater/{}", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(Duration::from_secs(10)))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            );
        if !use_proxy {
            b = b.proxy(None);
        }
        Self {
            agent: b.build().new_agent(),
            api_base: api_base.trim_end_matches('/').to_string(),
            repo: repo.to_string(),
            suffix: platform_suffix().to_string(),
        }
    }

    /// 配布物の名前の末尾を変える（テスト用）
    pub fn with_suffix(mut self, suffix: &str) -> Self {
        self.suffix = suffix.to_string();
        self
    }

    fn get_string(&self, url: &str, timeout: Duration) -> Result<String> {
        let mut resp = self
            .agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .config()
            .timeout_global(Some(timeout))
            .build()
            .call()
            .map_err(http_err)?;
        resp.body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| Error::Http(e.to_string()))
    }

    /// 最新のリリースを確かめる。`current` は手元の版（`CARGO_PKG_VERSION`）
    pub fn check(&self, current: &str) -> Result<Check> {
        let current = semver::Version::parse(current)
            .map_err(|e| Error::Parse(format!("手元の版 {current}: {e}")))?;
        let url = format!("{}/repos/{}/releases/latest", self.api_base, self.repo);
        let json = match self.get_string(&url, Duration::from_secs(20)) {
            Err(Error::Status(404)) => return Err(Error::NoRelease),
            r => r?,
        };
        let Some(rel) = parse_release(&json, &self.suffix)? else {
            return Ok(Check::UpToDate { latest: current });
        };
        if rel.version > current {
            Ok(Check::Available(rel))
        } else {
            Ok(Check::UpToDate {
                latest: rel.version,
            })
        }
    }

    /// zip を `dir` に取ってきて、SHA-256 を確かめる。`progress(読んだ量, 全体)` を途中で呼ぶ
    pub fn download(
        &self,
        rel: &Release,
        dir: &Path,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<PathBuf> {
        let zip = rel
            .zip
            .as_ref()
            .ok_or_else(|| Error::NoAsset(self.suffix.clone()))?;
        let sum = rel
            .checksum
            .as_ref()
            .ok_or_else(|| Error::NoAsset(format!("{}.sha256", zip.name)))?;
        let want = self.get_string(&sum.url, Duration::from_secs(30))?;
        let want = want
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();

        fs::create_dir_all(dir)?;
        let path = dir.join(&zip.name);
        let resp = self
            .agent
            .get(&zip.url)
            .config()
            .timeout_global(Some(Duration::from_secs(30 * 60)))
            .build()
            .call()
            .map_err(http_err)?;
        let mut reader = resp
            .into_body()
            .into_with_config()
            .limit(2 * 1024 * 1024 * 1024)
            .reader();
        let mut out = fs::File::create(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut done = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| Error::Http(e.to_string()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, zip.size.max(done));
        }
        out.flush()?;
        drop(out);
        let got = hex(&hasher.finalize());
        if got != want {
            let _ = fs::remove_file(&path);
            return Err(Error::Checksum);
        }
        Ok(path)
    }
}

fn http_err(e: ureq::Error) -> Error {
    match e {
        ureq::Error::StatusCode(c) => Error::Status(c),
        e => Error::Http(e.to_string()),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// ファイルの SHA-256（16進の小文字）
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// 展開済みの新しい版
#[derive(Debug)]
pub struct Staged {
    /// 展開したフォルダ（この下の相対パスがそのまま配布フォルダの相対パスになる）
    pub root: PathBuf,
    /// 置くファイルの相対パス
    pub files: Vec<PathBuf>,
}

/// 安全な相対パスか（`..` や絶対パスを含まない）
fn safe_relative(p: &Path) -> bool {
    !p.as_os_str().is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// zip を `dest` に展開する。中身が1つのフォルダにまとまっていれば、そのフォルダを根元にする
pub fn stage(zip_path: &Path, dest: &Path) -> Result<Staged> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;
    let f = fs::File::open(zip_path)?;
    let mut ar = zip::ZipArchive::new(f).map_err(|e| Error::Zip(e.to_string()))?;
    let mut names = Vec::new();
    for i in 0..ar.len() {
        let mut ent = ar.by_index(i).map_err(|e| Error::Zip(e.to_string()))?;
        let rel = match ent.enclosed_name() {
            Some(r) if safe_relative(&r) => r,
            _ => {
                return Err(Error::Zip(format!(
                    "危ないパス: {}",
                    ent.name().unwrap_or_default()
                )));
            }
        };
        let out = dest.join(&rel);
        if ent.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            fs::create_dir_all(p)?;
        }
        let mut w = fs::File::create(&out)?;
        std::io::copy(&mut ent, &mut w)?;
        #[cfg(unix)]
        if let Some(mode) = ent.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&out, fs::Permissions::from_mode(mode & 0o777));
        }
        names.push(rel);
    }
    // 1つのフォルダにまとまっているか
    let first = names
        .first()
        .and_then(|p| p.components().next())
        .map(|c| c.as_os_str().to_owned());
    let single = first.as_ref().is_some_and(|top| {
        names.iter().all(|p| {
            p.components().count() > 1
                && p.components().next().map(|c| c.as_os_str()) == Some(top.as_os_str())
        })
    });
    let (root, files) = match (single, first) {
        (true, Some(top)) => (
            dest.join(&top),
            names
                .iter()
                .map(|p| p.strip_prefix(&top).unwrap().to_path_buf())
                .collect(),
        ),
        _ => (dest.to_path_buf(), names),
    };
    if !files.iter().any(|f| f == Path::new(DIST_MARKER)) {
        return Err(Error::Zip(format!(
            "配布物の印 {DIST_MARKER} が入っていません"
        )));
    }
    Ok(Staged { root, files })
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// 置き換えたファイル
    pub replaced: usize,
    /// 新しく増えたファイル
    pub added: usize,
}

fn old_name(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

fn require_dist(install: &Path) -> Result<()> {
    if install.join(DIST_MARKER).is_file() {
        Ok(())
    } else {
        Err(Error::NotDist(
            install.join(DIST_MARKER).display().to_string(),
        ))
    }
}

/// 展開した新しい版を配布フォルダ `install` に入れる。途中で失敗したら、それまでに入れ替えた分を元に戻す
pub fn apply(staged: &Staged, install: &Path) -> Result<Applied> {
    require_dist(install)?;
    // (置いた先, 元のファイルを .old に退避したか)
    let mut done: Vec<(PathBuf, bool)> = Vec::new();
    let mut res = Applied::default();
    let result = (|| -> Result<()> {
        for rel in &staged.files {
            let src = staged.root.join(rel);
            let dst = install.join(rel);
            if let Some(p) = dst.parent() {
                fs::create_dir_all(p)?;
            }
            let existed = dst.exists();
            if existed {
                let old = old_name(&dst);
                if old.exists() {
                    fs::remove_file(&old)?;
                }
                fs::rename(&dst, &old)?;
            }
            if let Err(e) = fs::rename(&src, &dst).or_else(|_| fs::copy(&src, &dst).map(|_| ())) {
                if existed {
                    let _ = fs::rename(old_name(&dst), &dst);
                }
                return Err(e.into());
            }
            done.push((dst, existed));
            if existed {
                res.replaced += 1;
            } else {
                res.added += 1;
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        for (dst, existed) in done.into_iter().rev() {
            let _ = fs::remove_file(&dst);
            if existed {
                let _ = fs::rename(old_name(&dst), &dst);
            }
        }
        return Err(e);
    }
    Ok(res)
}

/// 前回の更新で残った `.old` と作業用フォルダを消す。消せたファイル数を返す
/// （まだ動いている別の exe の `.old` は消せないことがあるが、次の起動でまた試す）
pub fn cleanup_old(install: &Path) -> usize {
    fn walk(dir: &Path, n: &mut usize) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, n);
            } else if p.extension().is_some_and(|x| x == "old") && fs::remove_file(&p).is_ok() {
                *n += 1;
            }
        }
    }
    if require_dist(install).is_err() {
        return 0;
    }
    let mut n = 0;
    walk(install, &mut n);
    let _ = fs::remove_dir_all(install.join(WORK_DIR));
    n
}

/// 実行ファイルのあるフォルダ
pub fn install_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
}

/// 配布フォルダから動いているか（開発中の cargo run では false）
pub fn is_dist_install() -> bool {
    install_dir().is_some_and(|d| d.join(DIST_MARKER).is_file())
}

/// 取得・展開・適用をまとめて行う
pub fn download_and_apply(
    up: &Updater,
    rel: &Release,
    install: &Path,
    progress: impl FnMut(u64, u64),
) -> Result<Applied> {
    require_dist(install)?;
    let work = install.join(WORK_DIR);
    let zip = up.download(rel, &work, progress)?;
    let staged = stage(&zip, &work.join("staged"))?;
    let r = apply(&staged, install)?;
    let _ = fs::remove_dir_all(&work);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(parse_tag("v1.2.3"), Some(semver::Version::new(1, 2, 3)));
        assert_eq!(parse_tag("0.10.0"), Some(semver::Version::new(0, 10, 0)));
        assert_eq!(parse_tag("release-1"), None);
    }

    #[test]
    fn release_json() {
        let j = r#"{"tag_name":"v0.2.0","body":"変更点","html_url":"https://x/r","draft":false,"prerelease":false,
            "assets":[
              {"name":"janscope-0.2.0-windows-x64.zip","browser_download_url":"https://x/a.zip","size":10},
              {"name":"janscope-0.2.0-windows-x64.zip.sha256","browser_download_url":"https://x/a.sha","size":1},
              {"name":"janscope-0.2.0-linux-x64.zip","browser_download_url":"https://x/l.zip","size":10}
            ]}"#;
        let r = parse_release(j, "windows-x64.zip").unwrap().unwrap();
        assert_eq!(r.version, semver::Version::new(0, 2, 0));
        assert_eq!(r.zip.unwrap().url, "https://x/a.zip");
        assert_eq!(r.checksum.unwrap().url, "https://x/a.sha");
        let r = parse_release(j, "linux-x64.zip").unwrap().unwrap();
        assert!(r.checksum.is_none());
        let pre = j.replace("\"prerelease\":false", "\"prerelease\":true");
        assert!(parse_release(&pre, "windows-x64.zip").unwrap().is_none());
    }

    #[test]
    fn relative_paths() {
        assert!(safe_relative(Path::new("a/b.txt")));
        assert!(!safe_relative(Path::new("../a")));
        assert!(!safe_relative(Path::new("/etc/passwd")));
        assert!(!safe_relative(Path::new("")));
    }
}
