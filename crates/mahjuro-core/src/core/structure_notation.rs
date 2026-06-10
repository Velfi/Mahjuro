//! Compact structure notation for Cascade Lab and debug tooling.
//!
//! Tokens are whitespace-separated meld groups, e.g. `456s 789s www f1f2f3 ee`.
//! Numbered melds use consecutive ranks with a trailing suit letter (`m`/`p`/`s`).
//! Honors repeat their letter (`eee`, `www`); flowers use `f1`, `f2`, …

use crate::core::hand::{DetectedMeld, MeldKind};
use crate::core::tile::{Suit, Tile};

pub const STRUCTURE_NOTATION_HINT: &str = "e.g. 456s 789s www f1f2f3 ee";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructureNotationError(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Face {
    suit: Suit,
    rank: u8,
}

pub fn parse_structure_notation(
    input: &str,
) -> Result<(Vec<Tile>, Vec<DetectedMeld>), StructureNotationError> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.is_empty() {
        return Err(StructureNotationError("empty notation".into()));
    }

    let mut tiles = Vec::new();
    let mut sets = Vec::new();
    let mut next_id: u32 = 1;

    for token in tokens {
        let faces = parse_token(token)?;
        let kind = infer_meld_kind(&faces)?;
        let mut tile_ids = Vec::with_capacity(faces.len());
        for face in faces {
            let id = next_id;
            next_id += 1;
            tiles.push(Tile::new(face.suit, face.rank, id));
            tile_ids.push(id);
        }
        sets.push(DetectedMeld { kind, tile_ids });
    }

    Ok((tiles, sets))
}

pub fn format_structure_notation(tiles: &[Tile], sets: &[DetectedMeld]) -> String {
    sets.iter()
        .map(|set| format_meld_token(tiles, set))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_token(token: &str) -> Result<Vec<Face>, StructureNotationError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(StructureNotationError("empty token".into()));
    }
    if let Some(suit) = trailing_number_suit(token) {
        return parse_numbered_token(token, suit);
    }
    parse_mixed_token(token)
}

fn trailing_number_suit(token: &str) -> Option<Suit> {
    let last = token.chars().last()?;
    match last.to_ascii_lowercase() {
        'm' => Some(Suit::Manzu),
        'p' => Some(Suit::Pinzu),
        's' if token.len() > 1 => {
            let body = &token[..token.len() - 1];
            if body.chars().all(|c| c.is_ascii_digit()) {
                Some(Suit::Souzu)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn parse_numbered_token(token: &str, suit: Suit) -> Result<Vec<Face>, StructureNotationError> {
    let digits = &token[..token.len() - 1];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(StructureNotationError(format!(
            "invalid numbered token '{token}'"
        )));
    }
    let ranks: Vec<u8> = digits
        .chars()
        .map(|c| c.to_digit(10).unwrap() as u8)
        .collect();
    for &r in &ranks {
        if !(1..=9).contains(&r) {
            return Err(StructureNotationError(format!(
                "rank {r} out of range in '{token}'"
            )));
        }
    }
    Ok(ranks.into_iter().map(|rank| Face { suit, rank }).collect())
}

fn parse_mixed_token(token: &str) -> Result<Vec<Face>, StructureNotationError> {
    let lower = token.to_ascii_lowercase();
    let mut faces = Vec::new();
    let mut i = 0;
    let bytes = lower.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'f' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            let rank = bytes[i + 1] - b'0';
            if !(1..=4).contains(&rank) {
                return Err(StructureNotationError(format!(
                    "invalid flower rank in '{token}'"
                )));
            }
            faces.push(Face {
                suit: Suit::Flower,
                rank,
            });
            i += 2;
            continue;
        }
        if lower[i..].starts_with("wh") {
            faces.push(Face {
                suit: Suit::Dragon,
                rank: 3,
            });
            i += 2;
            continue;
        }
        let ch = bytes[i] as char;
        match ch {
            'e' => faces.push(Face {
                suit: Suit::Wind,
                rank: 1,
            }),
            's' => faces.push(Face {
                suit: Suit::Wind,
                rank: 2,
            }),
            'w' => faces.push(Face {
                suit: Suit::Wind,
                rank: 3,
            }),
            'n' => faces.push(Face {
                suit: Suit::Wind,
                rank: 4,
            }),
            'r' => faces.push(Face {
                suit: Suit::Dragon,
                rank: 1,
            }),
            'g' => faces.push(Face {
                suit: Suit::Dragon,
                rank: 2,
            }),
            _ => {
                return Err(StructureNotationError(format!(
                    "unrecognized segment in '{token}'"
                )));
            }
        }
        i += 1;
    }
    if faces.is_empty() {
        return Err(StructureNotationError(format!("empty token '{token}'")));
    }
    Ok(faces)
}

fn infer_meld_kind(faces: &[Face]) -> Result<MeldKind, StructureNotationError> {
    match faces.len() {
        2 => Ok(MeldKind::Pair),
        3 => {
            if faces.windows(2).all(|w| w[0] == w[1]) {
                Ok(MeldKind::Triplet)
            } else if is_consecutive_ranks(faces) {
                Ok(MeldKind::Sequence)
            } else {
                Ok(MeldKind::Triplet)
            }
        }
        4 | 5 => {
            if faces.windows(2).all(|w| w[0] == w[1]) {
                Ok(MeldKind::Kong)
            } else {
                Err(StructureNotationError(format!(
                    "cannot infer meld from {} tiles",
                    faces.len()
                )))
            }
        }
        n => Err(StructureNotationError(format!(
            "expected 2–5 tiles per token, got {n}"
        ))),
    }
}

fn is_consecutive_ranks(faces: &[Face]) -> bool {
    if faces.len() < 2 {
        return false;
    }
    let suit = faces[0].suit;
    if !faces.iter().all(|f| f.suit == suit) {
        return false;
    }
    for w in faces.windows(2) {
        if w[1].rank != w[0].rank + 1 {
            return false;
        }
    }
    true
}

fn format_meld_token(tiles: &[Tile], set: &DetectedMeld) -> String {
    let ordered: Vec<&Tile> = set
        .tile_ids
        .iter()
        .filter_map(|id| tiles.iter().find(|t| t.id == *id))
        .collect();
    if ordered.is_empty() {
        return "?".into();
    }

    match set.kind {
        MeldKind::Sequence if can_format_number_run(&ordered) => {
            let suit = ordered[0].suit;
            let mut s: String = ordered.iter().map(|t| t.rank.to_string()).collect();
            s.push(suit_letter(suit));
            s
        }
        MeldKind::Pair | MeldKind::Triplet | MeldKind::Kong
            if ordered
                .iter()
                .all(|t| t.suit == ordered[0].suit && t.rank == ordered[0].rank) =>
        {
            if ordered[0].is_number_tile() {
                let ch = ordered[0].rank.to_string();
                let mut s = String::new();
                for _ in &ordered {
                    s.push_str(&ch);
                }
                s.push(suit_letter(ordered[0].suit));
                s
            } else {
                ordered.iter().map(|t| honor_chunk(t)).collect::<String>()
            }
        }
        _ => ordered.iter().map(|t| honor_chunk(t)).collect::<String>(),
    }
}

fn can_format_number_run(tiles: &[&Tile]) -> bool {
    if tiles.len() < 3 {
        return false;
    }
    let suit = tiles[0].suit;
    if !matches!(suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) {
        return false;
    }
    tiles
        .windows(2)
        .all(|w| w[0].suit == suit && w[1].suit == suit && w[1].rank == w[0].rank + 1)
}

fn suit_letter(suit: Suit) -> char {
    match suit {
        Suit::Manzu => 'm',
        Suit::Pinzu => 'p',
        Suit::Souzu => 's',
        _ => '?',
    }
}

fn honor_chunk(tile: &Tile) -> String {
    match tile.suit {
        Suit::Wind => match tile.rank {
            1 => "e".into(),
            2 => "s".into(),
            3 => "w".into(),
            4 => "n".into(),
            _ => format!("w{}", tile.rank),
        },
        Suit::Dragon => match tile.rank {
            1 => "r".into(),
            2 => "g".into(),
            3 => "wh".into(),
            _ => format!("d{}", tile.rank),
        },
        Suit::Flower => format!("f{}", tile.rank),
        Suit::Season => format!("z{}", tile.rank),
        Suit::Manzu | Suit::Souzu | Suit::Pinzu => {
            format!("{}{}", tile.rank, suit_letter(tile.suit))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hand::MeldKind;

    fn faces(tiles: &[Tile]) -> Vec<(Suit, u8)> {
        tiles.iter().map(|t| (t.suit, t.rank)).collect()
    }

    #[test]
    fn parse_user_example() {
        let (tiles, sets) =
            parse_structure_notation("456s 789s www f1f2f3 ee").expect("example hand");
        assert_eq!(tiles.len(), 14);
        assert_eq!(sets.len(), 5);
        assert_eq!(sets[0].kind, MeldKind::Sequence);
        assert_eq!(sets[1].kind, MeldKind::Sequence);
        assert_eq!(sets[2].kind, MeldKind::Triplet);
        assert_eq!(sets[4].kind, MeldKind::Pair);
        assert_eq!(
            faces(&tiles)[0..3],
            [(Suit::Souzu, 4), (Suit::Souzu, 5), (Suit::Souzu, 6)]
        );
    }

    #[test]
    fn parse_eef1_triplet() {
        let (_, sets) = parse_structure_notation("eef1").expect("eef1");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, MeldKind::Triplet);
        assert_eq!(sets[0].tile_ids.len(), 3);
    }

    #[test]
    fn roundtrip_standard_win_shape() {
        let tiles = vec![
            Tile::new(Suit::Manzu, 1, 1),
            Tile::new(Suit::Manzu, 1, 2),
            Tile::new(Suit::Manzu, 2, 3),
            Tile::new(Suit::Manzu, 3, 4),
            Tile::new(Suit::Manzu, 4, 5),
            Tile::new(Suit::Pinzu, 2, 6),
            Tile::new(Suit::Pinzu, 3, 7),
            Tile::new(Suit::Pinzu, 4, 8),
            Tile::new(Suit::Souzu, 5, 9),
            Tile::new(Suit::Souzu, 6, 10),
            Tile::new(Suit::Souzu, 7, 11),
            Tile::new(Suit::Wind, 1, 12),
            Tile::new(Suit::Wind, 1, 13),
            Tile::new(Suit::Wind, 1, 14),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![6, 7, 8],
            },
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![12, 13, 14],
            },
        ];
        let text = format_structure_notation(&tiles, &sets);
        let (tiles2, sets2) = parse_structure_notation(&text).expect("roundtrip");
        assert_eq!(tiles.len(), tiles2.len());
        assert_eq!(sets.len(), sets2.len());
    }
}
