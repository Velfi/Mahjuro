//! Shared parser for `textures/tile_sets/<name>/atlas.toml`.

/// Parsed atlas metadata: `(tile_width, tile_height, columns, layout codes)`.
pub fn parse_atlas_toml(src: &str) -> Option<(u32, u32, u32, Vec<String>)> {
    let mut tile_w: Option<u32> = None;
    let mut tile_h: Option<u32> = None;
    let mut columns: Option<u32> = None;
    let mut layout: Vec<String> = Vec::new();

    let mut in_layout = false;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if in_layout {
            push_layout_tokens(line, &mut layout);
            if line.contains(']') {
                in_layout = false;
            }
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            match key {
                "tile_width" => tile_w = val.parse().ok(),
                "tile_height" => tile_h = val.parse().ok(),
                "columns" => columns = val.parse().ok(),
                "layout" => {
                    in_layout = true;
                    if let Some(rest) = val.strip_prefix('[') {
                        push_layout_tokens(rest, &mut layout);
                        if rest.contains(']') {
                            in_layout = false;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Some((tile_w?, tile_h?, columns?, layout))
}

fn push_layout_tokens(line: &str, out: &mut Vec<String>) {
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else { break };
        out.push(rest[..end].to_string());
        rest = &rest[end + 1..];
    }
}

#[cfg(test)]
mod tests {
    use super::parse_atlas_toml;

    #[test]
    fn parses_minimal_atlas_toml() {
        let src = r#"
tile_width = 100
tile_height = 200
columns = 9
layout = [
    "B1","B2","",
]
"#;
        let (w, h, cols, layout) = parse_atlas_toml(src).unwrap();
        assert_eq!(w, 100);
        assert_eq!(h, 200);
        assert_eq!(cols, 9);
        assert_eq!(layout, vec!["B1", "B2", ""]);
    }
}
