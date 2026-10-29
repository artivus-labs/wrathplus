//! A file under the loose AddOns root, sandboxed and matched whatever its case, as on Windows.

use std::path::{Path, PathBuf};

use wrathplus_ui::script::AddonReader;

/// `root/rel`, an escape or absolute path refused unread, each component matched whatever its case.
pub(super) fn read_under(root: &Path, rel: &str) -> Option<Vec<u8>> {
    if Path::new(rel)
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    let candidate = format!("interface\\addons\\{}", rel.replace('/', "\\"));
    std::fs::read(wrathplus_assets::loose_addon_file(root, &candidate)?).ok()
}

/// The AddOns folder at `root` as `register_addons`' reader, for `LoadAddOn` and `Bindings.xml`.
pub fn folder_reader(root: PathBuf) -> AddonReader {
    Box::new(move |rel| read_under(&root, rel))
}

#[cfg(test)]
mod tests {
    use super::super::{Addon, Source};
    use super::*;
    use wrathplus_ui::script::{AddOnInfo, UiScript};
    use wrathplus_ui::status::Status;
    use wrathplus_ui::toc::Toc;

    /// A fresh AddOns root under the temp folder, unique to the test and the process.
    fn temp_root(test: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("benilla-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("AddOns")).unwrap();
        tmp
    }

    /// A manifest entry, an `<Include>` and a `<Script file=>` open whatever their case.
    #[test]
    fn addon_files_open_whatever_the_case_of_the_reference() {
        let tmp = temp_root("addon-file-case");
        let root = tmp.join("AddOns");
        std::fs::create_dir_all(root.join("Probe/sub")).unwrap();
        std::fs::write(tmp.join("secret.txt"), "no").unwrap();
        std::fs::write(root.join("Probe/foo.lua"), "FOO = 1").unwrap();
        std::fs::write(
            root.join("Probe/sub/bar.xml"),
            r#"<Ui><Script file="BAZ.lua"/></Ui>"#,
        )
        .unwrap();
        std::fs::write(root.join("Probe/sub/baz.lua"), "BAZ = 1").unwrap();
        let toc = "## Interface: 30300\nFoo.lua\nSub\\Bar.xml\n";
        let addon = Addon {
            name: "Probe".into(),
            toc: Toc::parse(toc),
            source: Source::Dir(root.clone()),
        };

        let script = UiScript::new().unwrap();
        let failures = addon.load_wrath(&script, &mut Status::default());
        assert!(failures.is_empty(), "{failures:?}");
        assert!(script.eval::<bool>("return FOO == 1 and BAZ == 1").unwrap());
        // The exact spelling still reads, and the match never walks out of the AddOns root.
        assert_eq!(
            read_under(&root, "Probe/foo.lua").as_deref(),
            Some(&b"FOO = 1"[..])
        );
        assert!(read_under(&root, "../SECRET.TXT").is_none());
        assert!(read_under(&root, "PROBE/../../Secret.txt").is_none());
        assert!(read_under(&root, r"PROBE\..\..\Secret.txt").is_none());
        assert!(read_under(&root, "PROBE/..\\..\\Secret.txt").is_none());
        assert!(read_under(&root, tmp.join("secret.txt").to_str().unwrap()).is_none());
        assert!(read_under(&root, "PROBE/sub").is_none(), "not a file");
        assert!(read_under(&root, "PROBE/missing.lua").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `LoadAddOn` opens a LoadOnDemand addon's `Foo.lua` as `foo.lua`, and its `bindings.xml`.
    #[test]
    fn load_on_demand_opens_whatever_the_case_of_the_reference() {
        let tmp = temp_root("load-on-demand-case");
        let root = tmp.join("AddOns");
        std::fs::create_dir_all(root.join("Probe")).unwrap();
        std::fs::write(root.join("Probe/foo.lua"), "FOO = 1").unwrap();
        std::fs::write(
            root.join("Probe/bindings.xml"),
            r#"<Bindings><Binding name="PROBE_TOGGLE">PROBE = true</Binding></Bindings>"#,
        )
        .unwrap();
        let mut script = UiScript::new().unwrap();
        let info = AddOnInfo {
            name: "Probe".into(),
            interface: 30300,
            enabled: true,
            load_on_demand: true,
            files: vec!["Foo.lua".into()],
            ..Default::default()
        };
        script.register_addons(vec![info], Some(folder_reader(root)), None, None);
        script.note_addon_info_reply(&[]);
        assert!(script
            .eval::<bool>(
                "local loaded = LoadAddOn('Probe') == 1 \
                 local bound = false \
                 for i = 1, GetNumBindings() do \
                     if GetBinding(i) == 'PROBE_TOGGLE' then bound = true end \
                 end \
                 return loaded and FOO == 1 and bound"
            )
            .unwrap());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
