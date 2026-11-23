//! `slice init` project scaffolding command (`default` and `sveltekit` templates).

#[cfg(test)]
pub(crate) use super::init_templates::DEFAULT_SLICE_D_TS;
use super::init_templates::{
    DEFAULT_SAMPLE_INDEX_TS, DEFAULT_SAMPLE_TEST_TS, DEFAULT_SLICE_D_TS as SLICE_D_TS_TEMPLATE,
    DEFAULT_TSCONFIG_JSON, SVELTEKIT_ADAPTER_SLICE_JS, SVELTEKIT_API_ITEMS_SERVER_JS,
    SVELTEKIT_APP_HTML, SVELTEKIT_HOOKS_SERVER_JS, SVELTEKIT_PAGE_SERVER_JS, SVELTEKIT_PAGE_SVELTE,
    SVELTEKIT_VITE_CONFIG_JS, format_sveltekit_manifest_toml, format_sveltekit_package_json,
};
use anyhow::Result;
use slice_sandbox::SliceManifest;
use std::path::{Path, PathBuf};

fn write_if_missing(path: &Path, content: &str) -> Result<()> {
    if !path.exists() {
        std::fs::write(path, content)?;
    }
    Ok(())
}

fn init_sveltekit_template(target_dir: &Path, project_name: &str) -> Result<()> {
    std::fs::create_dir_all(
        target_dir
            .join("src")
            .join("routes")
            .join("api")
            .join("items"),
    )?;

    let manifest_path = target_dir.join("slice.toml");
    write_if_missing(
        &manifest_path,
        &format_sveltekit_manifest_toml(project_name),
    )?;

    let dts_path = target_dir.join("slice.d.ts");
    write_if_missing(&dts_path, SLICE_D_TS_TEMPLATE)?;

    let pkg_path = target_dir.join("package.json");
    write_if_missing(&pkg_path, &format_sveltekit_package_json(project_name))?;

    let adapter_path = target_dir.join("adapter-slice.js");
    write_if_missing(&adapter_path, SVELTEKIT_ADAPTER_SLICE_JS)?;

    let vite_cfg_path = target_dir.join("vite.config.js");
    write_if_missing(&vite_cfg_path, SVELTEKIT_VITE_CONFIG_JS)?;

    let app_html_path = target_dir.join("src").join("app.html");
    write_if_missing(&app_html_path, SVELTEKIT_APP_HTML)?;

    let hooks_path = target_dir.join("src").join("hooks.server.js");
    write_if_missing(&hooks_path, SVELTEKIT_HOOKS_SERVER_JS)?;

    let page_srv_path = target_dir
        .join("src")
        .join("routes")
        .join("+page.server.js");
    write_if_missing(&page_srv_path, SVELTEKIT_PAGE_SERVER_JS)?;

    let page_svelte_path = target_dir.join("src").join("routes").join("+page.svelte");
    write_if_missing(&page_svelte_path, SVELTEKIT_PAGE_SVELTE)?;

    let api_srv_path = target_dir
        .join("src")
        .join("routes")
        .join("api")
        .join("items")
        .join("+server.js");
    write_if_missing(&api_srv_path, SVELTEKIT_API_ITEMS_SERVER_JS)?;

    println!(
        "Initialized Slice SvelteKit (SvelteKit 3 + Svelte 5) project '{}' in {}",
        project_name,
        target_dir.display()
    );
    println!("  Created {}", manifest_path.display());
    println!("  Created {}", pkg_path.display());
    println!("  Created {}", adapter_path.display());
    println!("  Created {}", vite_cfg_path.display());
    println!("  Created {}", page_svelte_path.display());
    println!("  Created {}", api_srv_path.display());
    println!("Next steps:");
    println!("  npm install && npm run build");
    println!("  slice pack ./build/slice-entry.js ./build/app.cas --static=./build/client");
    println!("  slice serve ./build/app.cas --port=3000");
    Ok(())
}

pub(crate) fn cmd_init(args: &[String]) -> Result<()> {
    let mut target_dir = PathBuf::from(".");
    let mut template = String::from("default");
    let mut dir_set = false;
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if let Some(t) = arg.strip_prefix("--template=") {
            template = t.to_ascii_lowercase();
        } else if (arg == "--template" || arg == "-t") && i + 1 < args.len() {
            template = args[i + 1].to_ascii_lowercase();
            i += 1;
        } else if !arg.starts_with('-') && !dir_set {
            target_dir = PathBuf::from(arg);
            dir_set = true;
        }
        i += 1;
    }

    std::fs::create_dir_all(target_dir.join("src"))?;
    let project_name = target_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("slice-edge-app");

    if template == "sveltekit" || template == "svelte" {
        return init_sveltekit_template(&target_dir, project_name);
    }

    let manifest_path = target_dir.join("slice.toml");
    write_if_missing(
        &manifest_path,
        &SliceManifest::default_template(project_name),
    )?;

    let dts_path = target_dir.join("slice.d.ts");
    write_if_missing(&dts_path, SLICE_D_TS_TEMPLATE)?;

    let tsconfig_path = target_dir.join("tsconfig.json");
    write_if_missing(&tsconfig_path, DEFAULT_TSCONFIG_JSON)?;

    let entry_path = target_dir.join("src").join("index.ts");
    write_if_missing(&entry_path, DEFAULT_SAMPLE_INDEX_TS)?;

    let tests_dir = target_dir.join("tests");
    std::fs::create_dir_all(&tests_dir)?;
    let test_path = tests_dir.join("index.test.ts");
    write_if_missing(&test_path, DEFAULT_SAMPLE_TEST_TS)?;

    println!(
        "Initialized Slice (Nexus-JS) project '{}' in {}",
        project_name,
        target_dir.display()
    );
    println!("  Created {}", manifest_path.display());
    println!("  Created {}", dts_path.display());
    println!("  Created {}", tsconfig_path.display());
    println!("  Created {}", entry_path.display());
    println!("Next steps:");
    println!("  slice run                        (Execute with slice.toml OCap policy)");
    println!("  slice compile ./src/index.ts -o app  (Build standalone zero-dependency binary)");
    println!("  slice serve --port=3000          (Start multi-tenant HTTP MicroVM server)");
    Ok(())
}
