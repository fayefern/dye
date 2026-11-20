use toml::{Table, Value};

use crate::{
    error::AssetError,
    parse::{Input, ParseCx},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArrayMode {
    #[default]
    Replace,
    Append,
}

pub fn merge_toml(cx: &ParseCx, input: &Input, arrays: ArrayMode) -> Result<Table, AssetError> {
    let mut result = Table::new();
    let mut merged_any = false;
    let mut last_err = None;

    for entry in &input.entries {
        let parsed = entry.text().and_then(|s| {
            s.parse::<Table>()
                .map_err(|e| AssetError::Parse(e.message().to_owned()))
        });
        match parsed {
            Ok(table) => {
                merge_into(&mut result, table, arrays);
                merged_any = true;
            }
            Err(e) => {
                cx.warn(format!(
                    "`{}` in pack `{}`: {e}; layer ignored",
                    entry.sub_path, entry.pack
                ));
                last_err = Some(e);
            }
        }
    }

    if merged_any {
        Ok(result)
    } else {
        Err(last_err.unwrap_or_else(|| AssetError::Parse("no files to merge".into())))
    }
}

fn merge_into(dst: &mut Table, src: Table, arrays: ArrayMode) {
    for (key, val) in src {
        if let Some(slot) = dst.get_mut(&key) {
            match (slot, val) {
                (Value::Table(d), Value::Table(s)) => merge_into(d, s, arrays),
                (Value::Array(d), Value::Array(s)) if arrays == ArrayMode::Append => d.extend(s),
                (slot, val) => *slot = val,
            }
        } else {
            dst.insert(key, val);
        }
    }
}
