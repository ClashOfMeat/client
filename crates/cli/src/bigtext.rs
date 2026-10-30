//! Block letters for the link code, so it can be read from across the desk.

const ROWS: usize = 5;
const GAP: &str = " ";

/// `#` is a filled cell. Zero is slashed and D, B and 8 differ in their corners so the
/// hexadecimal code can't be misread.
fn glyph(c: char) -> Option<[&'static str; ROWS]> {
    Some(match c {
        '0' => [" ### ", "#  ##", "# # #", "##  #", " ### "],
        '1' => ["  #  ", " ##  ", "  #  ", "  #  ", " ### "],
        '2' => [" ### ", "#   #", "  ## ", " #   ", "#####"],
        '3' => ["#### ", "    #", " ### ", "    #", "#### "],
        '4' => ["#   #", "#   #", "#####", "    #", "    #"],
        '5' => ["#####", "#    ", "#### ", "    #", "#### "],
        '6' => [" ### ", "#    ", "#### ", "#   #", " ### "],
        '7' => ["#####", "    #", "   # ", "  #  ", "  #  "],
        '8' => [" ### ", "#   #", " ### ", "#   #", " ### "],
        '9' => [" ### ", "#   #", " ####", "    #", " ### "],
        'A' => [" ### ", "#   #", "#####", "#   #", "#   #"],
        'B' => ["#### ", "#   #", "#### ", "#   #", "#### "],
        'C' => [" ####", "#    ", "#    ", "#    ", " ####"],
        'D' => ["#### ", "#   #", "#   #", "#   #", "#### "],
        'E' => ["#####", "#    ", "#### ", "#    ", "#####"],
        'F' => ["#####", "#    ", "#### ", "#    ", "#    "],
        '-' => ["   ", "   ", "###", "   ", "   "],
        _ => return None,
    })
}

/// `ABCDE12345` → `ABCDE-12345`; other lengths are left as they are.
pub fn group(code: &str) -> String {
    if code.len() == 10 && code.is_ascii() {
        format!("{}-{}", &code[..5], &code[5..])
    } else {
        code.to_owned()
    }
}

/// The text in block letters, one string per row, or `None` when a character has no glyph.
pub fn render(text: &str) -> Option<Vec<String>> {
    let glyphs = text
        .chars()
        .map(|c| glyph(c.to_ascii_uppercase()))
        .collect::<Option<Vec<_>>>()?;
    if glyphs.is_empty() {
        return None;
    }
    Some(
        (0..ROWS)
            .map(|row| {
                glyphs
                    .iter()
                    .map(|glyph| glyph[row].replace('#', "█"))
                    .collect::<Vec<_>>()
                    .join(GAP)
            })
            .collect(),
    )
}

/// Columns the rendered text takes.
pub fn width(rows: &[String]) -> usize {
    rows.iter()
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALPHABET: &str = "0123456789ABCDEF-";

    #[test]
    fn every_glyph_is_rectangular() {
        for c in ALPHABET.chars() {
            let rows = glyph(c).unwrap();
            let width = rows[0].len();
            assert!(rows.iter().all(|row| row.len() == width), "glyph {c}");
            assert!(rows.iter().any(|row| row.contains('#')), "glyph {c}");
        }
    }

    #[test]
    fn glyphs_are_distinct() {
        let all: Vec<_> = ALPHABET.chars().map(|c| glyph(c).unwrap()).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn renders_a_grouped_code() {
        let rows = render(&group("0d8b0d8b0d")).unwrap();
        assert_eq!(rows.len(), ROWS);
        // Ten 5-wide glyphs, a 3-wide hyphen and ten gaps.
        assert_eq!(width(&rows), 10 * 5 + 3 + 10);
        assert!(rows.iter().all(|row| row.chars().count() == width(&rows)));
    }

    #[test]
    fn renders_exact_rows() {
        assert_eq!(
            render("1-A").unwrap(),
            [
                "  █        ███ ",
                " ██       █   █",
                "  █   ███ █████",
                "  █       █   █",
                " ███      █   █",
            ]
        );
    }

    #[test]
    fn rejects_unknown_characters_and_empty_text() {
        assert!(render("ABZ").is_none());
        assert!(render("").is_none());
    }

    #[test]
    fn groups_ten_character_codes_only() {
        assert_eq!(group("ABCDE12345"), "ABCDE-12345");
        assert_eq!(group("ABC"), "ABC");
    }
}
