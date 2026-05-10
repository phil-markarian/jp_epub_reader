use jp_core::{Error, Result};
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct JavaInfo {
    pub binary: PathBuf,
    pub version_string: String,
    pub major: u32,
}

pub fn detect() -> Result<JavaInfo> {
    let bin = which_java()
        .ok_or_else(|| Error::not_found("java not found on PATH"))?;
    let out = Command::new(&bin)
        .arg("-version")
        .output()
        .map_err(|e| Error::Other(format!("run java -version: {e}")))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first_line = stderr.lines().next().unwrap_or("").to_string();
    let major = parse_major(&first_line);
    Ok(JavaInfo {
        binary: bin,
        version_string: first_line,
        major,
    })
}

fn which_java() -> Option<PathBuf> {
    // 1. PATH lookup (covers the common case once Java is installed).
    if let Ok(out) = Command::new("/usr/bin/which").arg("java").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !s.is_empty() {
                let p = PathBuf::from(s);
                // The macOS stub binary at /usr/bin/java is the
                // "Install Java" launcher. Reject it.
                if p != std::path::Path::new("/usr/bin/java") {
                    return Some(p);
                }
            }
        }
    }
    // 2. Common Temurin install location on macOS.
    let candidates = [
        "/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home/bin/java",
        "/Library/Java/JavaVirtualMachines/temurin-21.jre/Contents/Home/bin/java",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn parse_major(version_line: &str) -> u32 {
    // e.g. `openjdk version "21.0.11" 2026-04-21 LTS`
    version_line
        .split('"')
        .nth(1)
        .and_then(|v| v.split('.').next())
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_major_modern() {
        assert_eq!(parse_major("openjdk version \"21.0.11\" 2026-04-21 LTS"), 21);
    }

    #[test]
    fn parse_major_oracle() {
        assert_eq!(
            parse_major("java version \"17.0.9\" 2023-10-17 LTS"),
            17
        );
    }

    #[test]
    fn parse_major_unknown() {
        assert_eq!(parse_major("???"), 0);
    }
}
