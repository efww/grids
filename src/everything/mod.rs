// Everything (voidtools), reached only after a subfolder search or the settings page asks.
// Message bytes, the search string and install-path rules are pure and tested here.
// The IPC window, the registry and winget stay in the runtime modules and are not touched at startup.
// Everything64.dll is never loaded.

mod ipc;
mod locate;
mod query;
mod run;
mod setup;

pub(crate) use locate::Presence;
pub(crate) use ipc::{ev_sort_click, ev_sort_col, ev_sort_ok, MAX_RESULTS};
pub(crate) use query::{forces_everywhere, search_text, without_scope_prefix};
pub(crate) use run::{running, search};
pub(crate) use setup::{not_installed, action, activity, bind_settings, busy, install_async, probe_async, shown, start_async, Activity, EvAction, Shown};
