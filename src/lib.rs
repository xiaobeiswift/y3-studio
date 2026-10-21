pub mod cli;
pub mod console;
pub mod custom_define;
pub mod eca_compiler;
pub mod editor_table;
pub mod env;
pub mod features;
pub mod gmp;
pub mod gui;
pub mod launcher;
pub mod meta_builder;
pub mod server;
pub mod utils;
pub mod workspace;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
