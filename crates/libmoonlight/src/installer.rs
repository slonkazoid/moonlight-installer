use super::types::{Branch, DetectedInstall, GitHubRelease, InstallInfo, MoonlightBranch};
use super::util::{get_download_dir, get_home_dir};
use crate::types::{
    FlatpakFilesystemOverride, FlatpakFilesystemOverridePermission, FlatpakOverrides,
};
use crate::{get_app_dir, get_local_share, get_moonlight_dir, DOWNLOAD_DIR, PATCHED_ASAR};
use std::path::PathBuf;

const USER_AGENT: &str =
    "moonlight-installer (https://github.com/moonlight-mod/moonlight-installer)";
const INSTALLED_VERSION_FILE: &str = ".moonlight-installed-version";

const GITHUB_REPO: &str = "moonlight-mod/moonlight";
const ARTIFACT_NAME: &str = "dist.tar.gz";
const NIGHTLY_REF_URL: &str = "https://moonlight-mod.github.io/moonlight/ref";
const NIGHTLY_DIST_URL: &str = "https://moonlight-mod.github.io/moonlight/dist.tar.gz";

pub struct Installer;

impl Default for Installer {
    fn default() -> Self {
        Self::new()
    }
}

impl Installer {
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }

    pub fn download_moonlight(&self, branch: MoonlightBranch) -> crate::Result<String> {
        let dir = get_download_dir();

        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }

        std::fs::create_dir_all(&dir)?;

        Ok(match branch {
            MoonlightBranch::Stable => self.download_stable(dir)?,
            MoonlightBranch::Nightly => self.download_nightly(dir)?,
        })
    }

    fn download_stable(&self, dir: PathBuf) -> crate::Result<String> {
        let release = self.get_stable_release()?;
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == ARTIFACT_NAME)
            .unwrap();

        let resp = reqwest::blocking::Client::new()
            .get(&asset.browser_download_url)
            .header("User-Agent", USER_AGENT)
            .send()?;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(resp));

        archive.unpack(dir)?;
        Ok(release.name)
    }

    fn download_nightly(&self, dir: PathBuf) -> crate::Result<String> {
        let version = self.get_nightly_version()?;
        let resp = reqwest::blocking::get(NIGHTLY_DIST_URL)?;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(resp));
        archive.unpack(dir)?;
        Ok(version)
    }

    pub fn get_latest_moonlight_version(&self, branch: MoonlightBranch) -> crate::Result<String> {
        match branch {
            MoonlightBranch::Stable => self.get_stable_release().map(|x| x.name),
            MoonlightBranch::Nightly => self.get_nightly_version(),
        }
    }

    pub fn get_downloaded_version(&self) -> crate::Result<Option<String>> {
        let dir = get_moonlight_dir();
        let version = std::fs::read_to_string(dir.join(INSTALLED_VERSION_FILE)).ok();
        Ok(version)
    }

    pub fn set_downloaded_version(&self, version: &str) -> crate::Result<()> {
        let dir = get_moonlight_dir();
        std::fs::write(dir.join(INSTALLED_VERSION_FILE), version)?;
        Ok(())
    }

    fn get_stable_release(&self) -> crate::Result<GitHubRelease> {
        let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
        let resp = reqwest::blocking::Client::new()
            .get(url)
            .header("User-Agent", USER_AGENT)
            .send()?
            .json()?;
        Ok(resp)
    }

    fn get_nightly_version(&self) -> crate::Result<String> {
        let resp = reqwest::blocking::get(NIGHTLY_REF_URL)?.text()?;
        Ok(resp
            .lines()
            .next()
            .map(ToString::to_string)
            .unwrap_or_default())
    }

    pub fn get_installs(&self) -> crate::Result<Vec<InstallInfo>> {
        self.detect_installs().map(|installs| {
            installs
                .into_iter()
                .map(|install| {
                    let patched = self.is_install_patched(&install).unwrap_or(false);
                    let has_config = install.branch.config().exists();

                    InstallInfo {
                        install,
                        patched,
                        has_config,
                    }
                })
                .collect()
        })
    }

    fn detect_installs(&self) -> crate::Result<Vec<DetectedInstall>> {
        match std::env::consts::OS {
            "windows" => {
                let appdata = std::env::var("LocalAppData").unwrap();
                let dirs = [
                    ("Discord", Branch::Stable),
                    ("DiscordPTB", Branch::PTB),
                    ("DiscordCanary", Branch::Canary),
                    ("DiscordDevelopment", Branch::Development),
                ];
                let mut installs = vec![];

                for (dir, branch) in dirs {
                    let path = PathBuf::from(appdata.clone()).join(dir);
                    if path.exists() {
                        // app-(version)
                        let mut app_dirs: Vec<_> = std::fs::read_dir(&path)?
                            .filter_map(Result::ok)
                            .filter(|x| x.file_name().to_string_lossy().starts_with("app-"))
                            .collect();

                        app_dirs.sort_by(|a, b| {
                            let a_file_name = a.file_name();
                            let b_file_name = b.file_name();
                            a_file_name.cmp(&b_file_name)
                        });

                        if let Some(most_recent_install) = app_dirs.last() {
                            installs.push(DetectedInstall {
                                branch,
                                path: most_recent_install.path(),
                                flatpak_id: None,
                            });
                        }
                    }
                }

                Ok(installs)
            }

            "macos" => {
                let apps_dirs = vec![
                    PathBuf::from("/Applications"),
                    get_home_dir().join("Applications"),
                ];

                let branches = [
                    ("Discord", Branch::Stable),
                    ("Discord PTB", Branch::PTB),
                    ("Discord Canary", Branch::Canary),
                    ("Discord Development", Branch::Development),
                ];

                let mut installs = vec![];

                for apps_dir in apps_dirs {
                    for (branch_name, branch) in branches {
                        let macos_app_dir = apps_dir.join(format!("{branch_name}.app"));

                        if !macos_app_dir.exists() {
                            continue;
                        }

                        let app_dir = macos_app_dir.join("Contents/Resources");

                        installs.push(DetectedInstall {
                            branch,
                            path: app_dir,
                            flatpak_id: None,
                        });
                    }
                }

                Ok(installs)
            }

            "linux" => {
                let local_share = get_local_share();
                let dirs = [
                    ("Discord", Branch::Stable, None),
                    ("DiscordPTB", Branch::PTB, None),
                    ("DiscordCanary", Branch::Canary, None),
                    ("DiscordDevelopment", Branch::Development, None),
                    // flatpak user installations
                    ("flatpak/app/com.discordapp.Discord/current/active/files/discord", Branch::Stable, Some("com.discordapp.Discord")),
                    ("flatpak/app/com.discordapp.DiscordCanary/current/active/files/discord-canary", Branch::Canary, Some("com.discordapp.DiscordCanary")),
                ];

                let mut installs = vec![];
                for (dir, branch, id) in dirs {
                    let path = local_share.join(dir);
                    if path.exists() {
                        installs.push(DetectedInstall {
                            branch,
                            path,
                            flatpak_id: id.map(Into::into),
                        });
                    }
                }

                Ok(installs)
            }

            _ => Ok(Vec::new()),
        }
    }

    // This will probably match other client mods that replace app.asar, but it
    // will just prompt them to unpatch, so I think it's fine
    fn is_install_patched(&self, install: &DetectedInstall) -> crate::Result<bool> {
        Ok(!get_app_dir(&install.path)?.join("app.asar").exists())
    }

    fn get_flatpak_overrides(&self, id: &str) -> crate::Result<Option<FlatpakOverrides>> {
        let overrides = get_local_share().join("flatpak").join("overrides");

        std::fs::create_dir_all(&overrides)?;

        let app_overrides = overrides.join(id);

        let file = match std::fs::OpenOptions::new().read(true).open(&app_overrides) {
            Ok(v) => v,
            Err(err) => match err.kind() {
                std::io::ErrorKind::NotFound => return Ok(None),
                _ => return Err(err.into()),
            },
        };

        serde_ini::from_read(file).or(Ok(None))
    }

    fn ensure_flatpak_overrides(&self, id: &str) -> crate::Result<()> {
        let overrides = self.get_flatpak_overrides(id)?;

        let has = overrides
            .as_ref()
            .and_then(|v| v.context.as_ref())
            .and_then(|v| v.filesystems.as_ref())
            .is_some_and(|v| {
                v.iter().any(|entry| {
                    entry.path == "xdg-config/moonlight-mod"
                        && entry.permission == FlatpakFilesystemOverridePermission::ReadWrite
                })
            });

        if has {
            return Ok(());
        }

        let mut overrides = overrides.unwrap_or_default();

        if overrides.context.is_none() {
            overrides.context = Some(Default::default());
        }
        let context = overrides.context.as_mut().unwrap();

        if context.filesystems.is_none() {
            context.filesystems = Some(Default::default());
        }
        let filesystem = context.filesystems.as_mut().unwrap();

        filesystem.push(FlatpakFilesystemOverride {
            path: String::from("xdg-config/moonlight-mod"),
            permission: FlatpakFilesystemOverridePermission::ReadWrite,
        });

        let app_overrides = get_local_share().join("flatpak").join("overrides").join(id);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .append(false)
            .open(&app_overrides)?;

        serde_ini::to_writer(&mut file, &overrides).expect("ini serialization to succeed");

        Ok(())
    }

    pub fn patch_install(
        &self,
        install: &DetectedInstall,
        override_download_dir: Option<PathBuf>,
    ) -> crate::Result<()> {
        let download_dir = override_download_dir.unwrap_or_else(get_download_dir);

        // TODO: flatpak and stuff
        let app_dir = get_app_dir(&install.path)?;
        let asar = app_dir.join("app.asar");
        std::fs::rename(&asar, asar.with_file_name(PATCHED_ASAR))?;
        std::fs::create_dir(app_dir.join("app"))?;

        let json = serde_json::json!({
          "name": install.branch.dashed_name(),
          "main": "./injector.js",
          "private": true
        });
        std::fs::write(app_dir.join("app/package.json"), json.to_string())?;

        let moonlight_injector = download_dir.join("injector.js");
        let injector = format!(
            r#"const MOONLIGHT_INJECTOR = {};
const PATCHED_ASAR = {};
const DOWNLOAD_DIR = {};
{}"#,
            serde_json::to_string(&moonlight_injector).unwrap(),
            serde_json::to_string(PATCHED_ASAR).unwrap(),
            serde_json::to_string(DOWNLOAD_DIR).unwrap(),
            include_str!("injector.js")
        );
        std::fs::write(app_dir.join("app/injector.js"), injector)?;

        if let Some(flatpak_id) = install.flatpak_id.as_deref() {
            self.ensure_flatpak_overrides(flatpak_id)?;
        }

        Ok(())
    }

    pub fn unpatch_install(&self, install: &DetectedInstall) -> crate::Result<()> {
        let app_dir = get_app_dir(&install.path)?;
        let asar = app_dir.join(PATCHED_ASAR);
        std::fs::rename(&asar, asar.with_file_name("app.asar"))?;
        std::fs::remove_dir_all(app_dir.join("app"))?;
        Ok(())
    }

    pub fn reset_config(&self, branch: Branch) {
        let config = branch.config();
        let new_name = format!(
            "{}-backup-{}.json",
            config.file_stem().unwrap().to_string_lossy(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );
        std::fs::rename(&config, config.with_file_name(new_name)).ok();
    }
}
