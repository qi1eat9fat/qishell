//! The folder a shell says it is in: OSC 7 (`file://host/path`, which the
//! setup ShellRS offers writes, and fish sends by itself) and iTerm2's OSC
//! 1337 `CurrentDir=`. `NoticeScanner` finds them in the output on the
//! parser thread; the right sidebar's 文件 follows them.

use percent_encoding::percent_decode_str;

/// A folder the terminal's output reported: where its shell is, if the
/// report came from this host and not from one ssh'd to inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportedDirectory {
    /// The host the report names; none when it names none (OSC 1337, an
    /// empty host, `localhost`).
    pub host: Option<String>,
    pub path: String,
}

/// The folder an OSC sequence reports, by its parameters as the parser
/// splits them on `;`, if it is a well-formed report.
pub(super) fn reported_directory(params: &[&[u8]]) -> Option<ReportedDirectory> {
    // A path may hold `;`, which the parser split on.
    let rest = || {
        params
            .get(1..)
            .unwrap_or_default()
            .iter()
            .map(|param| std::str::from_utf8(param).ok())
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(";"))
    };
    let directory = match *params.first()? {
        b"7" => {
            let url = rest()?;
            let scheme = url.get(..7)?;
            if !scheme.eq_ignore_ascii_case("file://") {
                return None;
            }
            let url = &url[7..];
            let slash = url.find('/')?;
            let (host, path) = url.split_at(slash);
            let path = percent_decode_str(path).decode_utf8().ok()?.into_owned();
            let host = (!host.is_empty() && !host.eq_ignore_ascii_case("localhost"))
                .then(|| host.to_string());
            ReportedDirectory { host, path }
        }
        b"1337" => ReportedDirectory {
            host: None,
            path: rest()?.strip_prefix("CurrentDir=")?.to_string(),
        },
        _ => return None,
    };
    (directory.path.starts_with('/') && !directory.path.chars().any(char::is_control))
        .then_some(directory)
}
