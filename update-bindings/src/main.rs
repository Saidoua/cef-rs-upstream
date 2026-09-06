#![doc = include_str!("../README.md")]

#[macro_use]
extern crate thiserror;

use clap::Parser;
use download_cef::DEFAULT_TARGET;
use std::{fs, io::Read, path::Path, sync::OnceLock};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Missing Parent")]
    MissingParent(std::path::PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Bindgen(#[from] bindgen::BindgenError),
    #[error(transparent)]
    Regex(#[from] regex::Error),
    #[error(transparent)]
    Syn(#[from] syn::Error),
    #[error("Parsing bindgen output failed")]
    Parse(#[from] parse_tree::Unrecognized),
    #[error("Missing Path")]
    MissingPath(std::path::PathBuf),
}

pub type Result<T> = std::result::Result<T, Error>;

mod dirs;
mod parse_tree;
mod resources;
mod upgrade;

fn default_version() -> &'static str {
    static DEFAULT_VERSION: OnceLock<String> = OnceLock::new();
    DEFAULT_VERSION
        .get_or_init(|| download_cef::default_version(env!("CARGO_PKG_VERSION")))
        .as_str()
}

fn default_download_url() -> &'static str {
    static DEFAULT_DOWNLOAD_URL: OnceLock<String> = OnceLock::new();
    DEFAULT_DOWNLOAD_URL
        .get_or_init(download_cef::default_download_url)
        .as_str()
}

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
struct Args {
    #[arg(short, long)]
    download: bool,
    #[arg(short, long)]
    bindgen: bool,
    #[arg(short, long, default_value = DEFAULT_TARGET)]
    target: String,
    #[arg(short, long, default_value = default_version())]
    version: String,
    #[arg(short, long, default_value = default_download_url())]
    mirror_url: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let target = args.target.as_str();

    if args.bindgen {
        if args.download {
            let _ = upgrade::download(args.mirror_url.as_str(), target, args.version.as_str());
        }

        upgrade::sys_bindgen(target)?;
    }

    let bindings_file = upgrade::get_target_bindings(target);
    let mut sys_bindings = dirs::get_sys_dir()?;
    sys_bindings.push("src");
    sys_bindings.push("bindings");
    sys_bindings.push(&bindings_file);
    let mut cef_bindings = dirs::get_cef_dir()?;
    cef_bindings.push("src");
    let mut cef_resources = cef_bindings.clone();
    cef_bindings.push("bindings");
    cef_bindings.push(&bindings_file);
    cef_resources.push("resources");
    cef_resources.push(&bindings_file);

    let bindings = parse_tree::generate_bindings(&sys_bindings)?;
    let source = read_bindings(&bindings)?;
    let dest = read_bindings(&cef_bindings).unwrap_or_default();

    if source != dest {
        fs::copy(&bindings, &cef_bindings)?;
        println!("Updated: {}", cef_bindings.display());
    }

    let resources = resources::generate_bindings(&sys_bindings)?;
    let source = read_bindings(&resources)?;
    let dest = read_bindings(&cef_resources).unwrap_or_default();

    if source != dest {
        fs::copy(&resources, &cef_resources)?;
        println!("Updated: {}", cef_resources.display());
    }

    Ok(())
}

fn read_bindings(source_path: &Path) -> crate::Result<String> {
    let mut source_file = fs::File::open(source_path)?;
    let mut updated = String::default();
    source_file.read_to_string(&mut updated)?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    #[test]
    fn struct_only_wrap_macro_allows_external_trait_impls() {
        let source = r#"
            use std::ffi::c_void;

            mod rc {
                use super::c_void;

                pub trait Rc {
                    fn add_ref(&self) {}
                }

                pub trait WrapRcPtr {
                    fn wrap_rc_ptr(&self) -> *mut c_void;
                }

                pub struct RcImpl<T, I> {
                    pub cef_object: T,
                    pub interface: I,
                }
            }

            mod sys {
                #[allow(non_camel_case_types)]
                pub struct cef_base_ref_counted_t;
                pub struct ViewDelegate;
                pub struct PanelDelegate;
                pub struct WindowDelegate;
            }

            use rc::{Rc, RcImpl, WrapRcPtr};

            trait ImplViewDelegate: Clone + Sized + Rc + WrapRcPtr {
                fn get_raw(&self) -> *mut sys::ViewDelegate {
                    self.wrap_rc_ptr().cast()
                }
            }

            trait ImplPanelDelegate: ImplViewDelegate {
                fn get_raw(&self) -> *mut sys::PanelDelegate {
                    <Self as ImplViewDelegate>::get_raw(self).cast()
                }
            }

            trait ImplWindowDelegate: ImplPanelDelegate {}

            trait WrapWindowDelegate: ImplWindowDelegate {
                fn wrap_rc(&mut self, object: *mut RcImpl<sys::WindowDelegate, Self>);
            }

            macro_rules! wrap_window_delegate {
                ($vis:vis struct $name:ident;) => {
                    wrap_window_delegate! {
                        $vis struct $name {}
                    }
                };
                ($vis:vis struct $name:ident { $($field_name:ident: $field_type:ty),* $(,)? }) => {
                    $vis struct $name {
                        $($field_name: $field_type,)*
                        cef_object: *mut RcImpl<sys::WindowDelegate, Self>,
                    }

                    impl $name {
                        pub fn new($($field_name: $field_type),*) -> Self {
                            Self {
                                $($field_name,)*
                                cef_object: std::ptr::null_mut(),
                            }
                        }
                    }

                    impl WrapWindowDelegate for $name {
                        fn wrap_rc(&mut self, cef_object: *mut RcImpl<sys::WindowDelegate, Self>) {
                            self.cef_object = cef_object;
                        }
                    }

                    impl Clone for $name {
                        fn clone(&self) -> Self {
                            Self {
                                $($field_name: self.$field_name.clone(),)*
                                cef_object: self.cef_object,
                            }
                        }
                    }

                    impl Rc for $name {}

                    impl WrapRcPtr for $name {
                        fn wrap_rc_ptr(&self) -> *mut c_void {
                            self.cef_object.cast()
                        }
                    }
                };
            }

            wrap_window_delegate! {
                struct DemoWindowDelegate;
            }

            impl ImplViewDelegate for DemoWindowDelegate {}
            impl ImplPanelDelegate for DemoWindowDelegate {}
            impl ImplWindowDelegate for DemoWindowDelegate {}

            fn assert_wrap<T: WrapWindowDelegate>() {}

            fn main() {
                assert_wrap::<DemoWindowDelegate>();
            }
        "#;

        let test_dir =
            std::env::temp_dir().join(format!("cef-rs-wrap-macro-test-{}", std::process::id()));
        fs::create_dir_all(&test_dir).unwrap();
        let source_path = test_dir.join("main.rs");
        fs::write(&source_path, source).unwrap();

        let output = Command::new("rustc")
            .arg("--edition=2021")
            .arg(&source_path)
            .arg("--out-dir")
            .arg(&test_dir)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "rustc failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
