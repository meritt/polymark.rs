use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

fn main() {
    let mut ranges: Vec<(u32, u32)> = Vec::new();

    for point in 0..=0x10_ffff_u32 {
        let Some(char) = char::from_u32(point) else {
            continue;
        };
        let sign =
            matches!(char.general_category_group(), GeneralCategoryGroup::Punctuation | GeneralCategoryGroup::Symbol);

        if !sign {
            continue;
        }

        match ranges.last_mut() {
            Some((_, end)) if *end + 1 == point => *end = point,
            _ => ranges.push((point, point)),
        }
    }

    println!("const RANGES: [(u32, u32); {}] = [", ranges.len());

    for chunk in ranges.chunks(6) {
        let line: Vec<String> = chunk.iter().map(|(start, end)| format!("(0x{start:x}, 0x{end:x})")).collect();

        println!("    {},", line.join(", "));
    }

    println!("];");
}
