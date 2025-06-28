use std::{
    fmt::Display,
    path::PathBuf,
};

use chrono::Utc;
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub struct PrettyJsonError {
    #[source]
    source: serde_json::Error,
    pretty: Option<(usize, usize, String)>,
}

impl PrettyJsonError {
    pub fn pretty_json(&self) -> Option<&str> {
        self.pretty.as_ref().map(|(_, _, json)| json.as_str())
    }

    pub fn dump_pretty_json(&self) -> Option<PathBuf> {
        if let Some(json) = self.pretty_json() {
            let path = PathBuf::from(format!(
                "json_decode_error_{}.json",
                Utc::now().format("%+")
            ));
            std::fs::write(&path, json).unwrap();
            Some(path)
        }
        else {
            None
        }
    }
}

impl Display for PrettyJsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", &self.source)?;

        if let Some((line, col, pretty)) = &self.pretty {
            for (line_num, line_str) in pretty.lines().enumerate() {
                if line.abs_diff(line_num) < 5 {
                    writeln!(f, "{:>4} {line_str}", line_num + 1)?;
                }
                if *line == line_num {
                    struct Dashes(usize);
                    impl Display for Dashes {
                        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                            for _ in 0..self.0 {
                                write!(f, "-")?;
                            }
                            Ok(())
                        }
                    }
                    writeln!(f, "     {}^", Dashes(col.saturating_sub(1)))?;
                }
            }
        }

        Ok(())
    }
}

pub fn json_decode<T: DeserializeOwned>(json: impl AsRef<[u8]>) -> Result<T, PrettyJsonError> {
    let json = json.as_ref();
    serde_json::from_slice(json).map_err(|source| {
        let json: serde_json::Value = serde_json::from_slice(json).unwrap();
        let pretty_json = serde_json::to_string_pretty(&json).unwrap();
        match serde_json::from_str::<T>(&pretty_json) {
            Ok(_) => panic!("pretty printed JSON parsed successfully"),
            Err(error) => {
                PrettyJsonError {
                    source,
                    pretty: Some((
                        error.line() - 1,
                        error.column().saturating_sub(1),
                        pretty_json,
                    )),
                }
            }
        }
    })
}
