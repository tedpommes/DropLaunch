pub mod appdata;
pub mod archive;
pub mod assets;
pub mod client;
pub mod commands;
pub mod concmd;
pub mod config;
pub mod http;
pub mod jobs;
pub mod kits;
pub mod lang;
pub mod log;
pub mod maps;
pub mod mods;
pub mod owner;
pub mod platform;
pub mod playtest;
pub mod server;
pub mod startfile;
pub mod steamcmd;
pub mod sync;
pub mod theme;
pub mod toast;
pub mod updater;
pub mod watcher;
pub mod whitelist;

use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};

/// Command-line arguments that are maps to open (a .map/.zip/folder dropped on the exe), language,
/// theme or kit files (.json), or start files (.bat/.cmd/.sh) to add.
fn openable(args: &[String], cwd: &Path) -> Vec<String> {
    args.iter()
        .filter(|a| !a.starts_with('-'))
        .map(|a| {
            let p = PathBuf::from(a);
            if p.is_absolute() {
                p
            } else {
                cwd.join(p)
            }
        })
        .filter(|p| {
            p.is_dir() || startfile::is_script(p) || p.extension().is_some_and(|e| ["map", "zip", "json"].iter().any(|x| e.eq_ignore_ascii_case(x)))
        })
        .map(|p| p.display().to_string())
        .collect()
}

pub fn run() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Started by an update: the old DropLaunch closes first (only one runs at a time).
    updater::after_update(&mut args);
    let cwd = std::env::current_dir().unwrap_or_default();
    let files = openable(&args, &cwd);

    let mut builder = tauri::Builder::default();
    // A dev build pointed at another folder (DROPLAUNCH_HOME) runs next to the real one.
    if std::env::var_os("DROPLAUNCH_HOME").is_none() {
        // Must be registered first. A second launch (e.g. a .map dropped on the exe while
        // DropLaunch is open) is handed to this window instead of starting another app.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            let files = openable(argv.get(1..).unwrap_or_default(), Path::new(&cwd));
            if !files.is_empty() {
                let _ = app.emit("open-files", files);
            }
        }));
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            // Loaded here, after the plugins: a second launch has already been handed to the
            // DropLaunch that runs (and has exited) before it reads or writes anything.
            app.manage(commands::AppState::load(files));
            let state = app.state::<commands::AppState>();
            // Made here, not from tauri.conf.json: its storage (WebView2's) goes into the data folder.
            let config = app.config().app.windows.iter().find(|w| w.label == "main").ok_or("no main window in tauri.conf.json")?;
            let window = tauri::WebviewWindowBuilder::from_config(app.handle(), config)?.data_directory(state.webview_dir()).build()?;
            state.dress_window(&window);
            state.write_update_scripts();
            state.server.run(app.handle().clone());
            state.playtest.run(app.handle().clone());
            state.watcher.run(app.handle().clone());
            state.watcher.watch(state.maps_dir());
            // Up and running: the version before an update isn't needed any more.
            updater::remove_leftovers();
            // Once, in the background: map backups past their days (otherwise only pruned when
            // another map is backed up) and map pictures not shown for a month.
            let data = state.data_dir();
            let _ = std::thread::Builder::new().name("housekeeping".into()).spawn(move || {
                client::prune_backups(&data.join(client::BACKUP_DIR));
                playtest::mapview::prune_thumbnails(&data.join(playtest::mapview::CACHE_DIR));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::set_folder,
            commands::pick_folder,
            commands::open_folder,
            commands::reveal,
            commands::set_branch,
            commands::set_options,
            commands::quick_branches,
            commands::fetch_branches,
            commands::server_update,
            commands::reapply_oxide,
            commands::setup_all,
            commands::branch_folder,
            commands::show_rust,
            commands::restart_rust_and_join,
            commands::install_rust_game,
            commands::console_commands,
            commands::console_history,
            commands::console_history_add,
            commands::server_space,
            commands::cancel_job,
            commands::list_maps,
            commands::import_files,
            commands::forget_map,
            commands::take_pending_files,
            commands::start_server,
            commands::stop_server,
            commands::restart_server,
            commands::server_command,
            commands::server_status,
            commands::create_start_file,
            commands::join_server,
            commands::steam_accounts,
            commands::steam_avatar,
            commands::kits_load,
            commands::kits_save,
            commands::kit_items,
            commands::item_icon,
            commands::kit_players,
            commands::kit_give,
            commands::kits_import,
            commands::kits_built_in,
            commands::reset_settings,
            commands::bug_reports_summary,
            commands::bug_reports_clear,
            commands::assets_list,
            commands::assets_forget,
            commands::start_files_add,
            commands::start_file_use,
            commands::start_file_forget,
            commands::server_ask,
            commands::set_owner,
            commands::update_setup_options,
            commands::open_link,
            commands::client_cache,
            commands::clear_client_cache,
            commands::notify,
            commands::check_update,
            commands::install_update,
            commands::playtest_status,
            commands::playtest_set_fixed,
            commands::playtest_delete,
            commands::playtest_teleport,
            commands::playtest_picture,
            commands::playtest_set_enabled,
            commands::playtest_save_report,
            commands::playtest_teleport_to,
            commands::map_thumbnail,
            commands::map_picture,
            commands::map_notes,
            commands::language_state,
            commands::set_language,
            commands::add_languages,
            commands::open_languages_folder,
            commands::export_language,
            commands::theme_state,
            commands::set_theme,
            commands::add_themes,
            commands::remove_theme,
            commands::open_themes_folder,
            commands::save_theme,
        ])
        .build(tauri::generate_context!())
        .expect("error while building DropLaunch")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<commands::AppState>() {
                    state.server.shutdown();
                }
            }
        });
}
