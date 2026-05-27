use serde::de::DeserializeOwned;

pub fn load_json_asset<T: DeserializeOwned>(path: &str, missing_what: &str) -> T {
    let bytes = mahjuro_assets::asset_path::get(path)
        .unwrap_or_else(|| panic!("{missing_what} file missing: assets/{path}"));
    serde_json::from_slice(&bytes.data)
        .unwrap_or_else(|e| panic!("failed to parse assets/{path}: {e}"))
}
