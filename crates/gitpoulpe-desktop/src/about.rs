const FONT_FILES: [&str; 5] = [
    "IBMPlexSans-Regular.ttf",
    "IBMPlexSans-Medium.ttf",
    "IBMPlexSans-SemiBold.ttf",
    "IBMPlexMono-Regular.ttf",
    "IBMPlexMono-Medium.ttf",
];

const SANS_LICENSE: &str = include_str!("../assets/fonts/LICENSE-IBM-Plex-Sans.txt");
const MONO_LICENSE: &str = include_str!("../assets/fonts/LICENSE-IBM-Plex-Mono.txt");

pub fn version_text() -> String {
    let commit = option_env!("GITPOULPE_BUILD_COMMIT").unwrap_or("unknown");
    format!(
        "gitpoulpe {} (commit {commit})\nLicense: {}\n",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_LICENSE")
    )
}

pub fn licenses_text() -> String {
    format!(
        "{version}\nGitPoulpe is free software released under the GNU General Public License, version 3 only. It comes with no warranty. The full text is in the LICENSE file shipped with the package and at https://www.gnu.org/licenses/gpl-3.0.html.\n\nEmbedded fonts (IBM Plex, from https://github.com/IBM/plex, SIL Open Font License 1.1):\n{fonts}\n--- IBM Plex Sans ---\n{sans}\n--- IBM Plex Mono ---\n{mono}\n",
        version = version_text(),
        fonts = FONT_FILES
            .iter()
            .map(|f| format!("  {f}\n"))
            .collect::<String>(),
        sans = SANS_LICENSE.trim_end(),
        mono = MONO_LICENSE.trim_end(),
    )
}

pub fn text_for_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Option<String> {
    for arg in args.into_iter().skip(1) {
        match arg.to_str() {
            Some("--version" | "-V") => return Some(version_text()),
            Some("--licenses") => return Some(licenses_text()),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<std::ffi::OsString> {
        v.iter().map(std::ffi::OsString::from).collect()
    }

    #[test]
    fn the_listed_fonts_are_exactly_the_embedded_ones() {
        assert_eq!(FONT_FILES.len(), crate::theme::FONTS.len());
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        for (name, embedded) in FONT_FILES.iter().zip(crate::theme::FONTS) {
            assert_eq!(&std::fs::read(dir.join(name)).unwrap(), embedded, "{name}");
        }
    }

    #[test]
    fn version_and_licenses_expose_version_licence_and_font_provenance() {
        let version = version_text();
        assert!(version.contains(env!("CARGO_PKG_VERSION")) && version.contains("GPL-3.0-only"));
        let licenses = licenses_text();
        for needle in [
            "General Public License, version 3",
            "SIL Open Font License",
            "github.com/IBM/plex",
            "IBMPlexMono-Medium.ttf",
            "IBM Plex Sans ---",
        ] {
            assert!(licenses.contains(needle), "{needle}");
        }
    }

    #[test]
    fn only_the_information_flags_short_circuit_the_application() {
        assert_eq!(
            text_for_args(args(&["gitpoulpe", "--version"])),
            Some(version_text())
        );
        assert_eq!(
            text_for_args(args(&["gitpoulpe", "-V"])),
            Some(version_text())
        );
        assert_eq!(
            text_for_args(args(&["gitpoulpe", "--licenses"])),
            Some(licenses_text())
        );
        assert_eq!(text_for_args(args(&["gitpoulpe", "/tmp/repo"])), None);
        assert_eq!(text_for_args(args(&["--version"])), None);
    }
}
