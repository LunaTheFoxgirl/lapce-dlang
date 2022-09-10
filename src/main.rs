use std::{
    fs::{self, create_dir_all},
    path::PathBuf,
};

use anyhow::{Error, Result};
use lapce_plugin::{
    psp_types::{
        lsp_types::{request::Initialize, InitializeParams, Url},
        Request,
    },
    register_plugin, LapcePlugin, VoltEnvironment, PLUGIN_RPC,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Cursor;
use tar_wasi::Archive;
use zip::ZipArchive;

#[derive(Default)]
struct State {}

register_plugin!(State);

const LANGUAGE_ID: &str = "d";

#[derive(Serialize, Deserialize)]
struct GHAsset {
    tag_name: String,
    assets: Vec<GHReleaseAsset>,
}

#[derive(Serialize, Deserialize)]
struct GHReleaseAsset {
    id: isize,
    name: String,
    size: isize,
    download_count: isize,
    browser_download_url: String,
    created_at: String,
}

fn initialize(params: InitializeParams) -> Result<()> {
    let mut server_args = vec!["--require".to_string(), "d".to_string()];
    let mut installed_version = semver::Version::parse("v0.0.0")?;

    // Check for user specified LSP server path
    // ```
    // [lapce-plugin-name.lsp]
    // serverPath = "[path or filename]"
    // serverArgs = ["--arg1", "--arg2"]
    // ```
    if let Some(options) = params.initialization_options.as_ref() {
        if let Some(lsp) = options.get("lsp") {
            if let Some(args) = lsp.get("serverArgs") {
                if let Some(args) = args.as_array() {
                    for arg in args {
                        if let Some(arg) = arg.as_str() {
                            server_args.push(arg.to_string());
                        }
                    }
                }
            }

            // Allow starting specific LSP
            if let Some(server_path) = lsp.get("serverPath") {
                if let Some(server_path) = server_path.as_str() {
                    if !server_path.is_empty() {
                        PLUGIN_RPC.start_lsp(
                            Url::parse(&format!("urn:{}", server_path))?,
                            server_args,
                            LANGUAGE_ID,
                            params.initialization_options,
                        );
                        return Ok(());
                    }
                }
            }
        }
    }

    PLUGIN_RPC.stderr("AAAAAAAA");

    // Fetch asset from github to check version
    let asset: GHAsset = serde_json::from_str(
        String::from_utf8(
            lapce_plugin::Http::get("https://api.github.com/repos/Pure-D/serve-d/releases/latest")?
                .body_read_all()?,
        )?
        .as_str(),
    )?;

    // Architecture check
    let arch_name = match VoltEnvironment::architecture().as_deref() {
        Ok("x86_64") => "x86_64",
        Ok("aarch64") => "arm64",
        _ => return Err(Error::msg("Unsupported architecture")),
    };

    // OS check
    let os_name = match VoltEnvironment::operating_system().as_deref() {
        Ok("macos") => "macos",
        Ok("linux") => "linux",
        Ok("windows") => "windows",
        _ => return Err(Error::msg("Unsupported platform")),
    };

    // see lapce_plugin::Http for available API to download files

    let exec_file = match VoltEnvironment::operating_system().as_deref() {
        Ok("windows") => {
            format!("{}.exe", "serve-d")
        }
        _ => "serve-d".to_string(),
    };

    // Plugin working directory
    let volt_uri = VoltEnvironment::uri()?;
    let server_path = Url::parse(&volt_uri)?.join(exec_file.as_str())?;
    let verfile = PathBuf::from(format!("{0}/{1}", volt_uri, "version.txt"));

    let mut should_update: bool;

    // Create server path if it doesn't already exist
    if !PathBuf::from(volt_uri.as_str()).exists() {
        create_dir_all(volt_uri.as_str())?;
        should_update = true;

        // Create version file (it definitely doesn't exist)
        fs::write(&verfile, &asset.tag_name)?;
    } else {
        if verfile.exists() {
            // Get version from file if there is one
            let ver = String::from_utf8(fs::read(&verfile)?)?;
            installed_version = semver::Version::parse(ver.as_str())?;
        }

        // Write the new version we want.
        fs::write(&verfile, &asset.tag_name)?;

        // Set should_update based on whether the version on git is newer
        should_update = installed_version > semver::Version::parse(asset.tag_name.as_str())?;
    }

    if should_update {
        let ext = if os_name == "windows" {
            "zip"
        } else {
            "tar.xz"
        };

        // Calculate download url
        let download_url = format!(
            "https://github.com/Pure-D/serve-d/releases/download/{0}/serve-d_{0}-{1}-{2}.{3}",
            asset.tag_name.clone(),
            arch_name,
            os_name,
            ext
        );

        // Try fetching the archive
        let mut resp = lapce_plugin::Http::get(download_url.as_str())?;
        if resp.status_code != 200 {
            return Err(Error::msg(format!(
                "Fetching archive failed with error {}",
                resp.status_code
            )));
        }

        // Archive buffer
        let archive_buf = resp.body_read_all()?;

        // Extract zip or tar archive
        match ext {
            "zip" => {
                let mut archive =
                    ZipArchive::new(Cursor::new(archive_buf)).expect("Failed to open zip archive");
                archive.extract(volt_uri.as_str())?;
            }
            "tar.xz" => {
                let mut archive = Archive::new(Cursor::new(archive_buf));
                archive.unpack(volt_uri.as_str())?;
            }
            _ => {}
        }
    }

    // Available language IDs
    // https://github.com/lapce/lapce/blob/HEAD/lapce-proxy/src/buffer.rs#L173
    PLUGIN_RPC.start_lsp(
        server_path,
        server_args,
        LANGUAGE_ID,
        params.initialization_options,
    );

    Ok(())
}

impl LapcePlugin for State {
    fn handle_request(&mut self, _id: u64, method: String, params: Value) {
        #[allow(clippy::single_match)]
        match method.as_str() {
            Initialize::METHOD => {
                let params: InitializeParams = serde_json::from_value(params).unwrap();
                let _ = initialize(params);
            }
            _ => {}
        }
    }
}
