#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Start,
    Bare,
    Quoted,
    Closed,
}

pub fn delimiter(text: &str) -> char {
    let mut counts = [0usize; 3];
    let mut quoted = false;
    for ch in text.trim_start_matches('\u{feff}').chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if !quoted {
            match ch {
                ',' => counts[0] += 1,
                ';' => counts[1] += 1,
                '\t' => counts[2] += 1,
                '\r' | '\n' => break,
                _ => (),
            }
        }
    }
    if counts[2] > counts[0].max(counts[1]) {
        '\t'
    } else if counts[1] > counts[0] {
        ';'
    } else {
        ','
    }
}

pub fn parse(
    text: &str,
    delimiter: char,
    max_rows: usize,
    max_columns: usize,
) -> Result<Vec<Vec<String>>, String> {
    let mut state = Field::Start;
    let mut field = String::new();
    let mut row = Vec::new();
    let mut rows = Vec::new();
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(ch) = chars.next() {
        if state == Field::Quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    state = Field::Closed;
                }
            } else {
                field.push(ch);
            }
        } else if ch == delimiter || matches!(ch, '\r' | '\n') {
            push_field(&mut row, &mut field, max_columns)?;
            if ch != delimiter {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                push_row(&mut rows, &mut row, max_rows)?;
            }
            state = Field::Start;
        } else if ch == '"' && state == Field::Start {
            state = Field::Quoted;
        } else {
            if ch == '"' || state == Field::Closed {
                return Err("The table contains an invalid quoted field.".into());
            }
            field.push(ch);
            state = Field::Bare;
        }
        if field.len() > 8192 {
            return Err("A table field exceeds its size limit.".into());
        }
    }
    if state == Field::Quoted {
        return Err("The table contains an unterminated quoted field.".into());
    }
    if state != Field::Start || !row.is_empty() {
        push_field(&mut row, &mut field, max_columns)?;
        push_row(&mut rows, &mut row, max_rows)?;
    }
    Ok(rows)
}

fn push_field(row: &mut Vec<String>, value: &mut String, limit: usize) -> Result<(), String> {
    if row.len() >= limit {
        return Err("The table exceeds its column limit.".into());
    }
    row.push(std::mem::take(value));
    Ok(())
}

fn push_row(
    rows: &mut Vec<Vec<String>>,
    row: &mut Vec<String>,
    limit: usize,
) -> Result<(), String> {
    let record = std::mem::take(row);
    if record.len() == 1 && record[0].trim().is_empty() {
        return Ok(());
    }
    if rows.len() >= limit {
        return Err("The table exceeds its row limit.".into());
    }
    rows.push(record);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreadsheet_quotes_multiline_and_empty_trailing_values() {
        let text =
            "\u{feff}Name;Note;Version\r\n\"Track; 1\";\"First\r\nSecond \"\"quote\"\"\";\r\n";
        assert_eq!(delimiter(text), ';');
        assert_eq!(
            parse(text, ';', 2, 3).unwrap(),
            vec![
                vec!["Name", "Note", "Version"],
                vec!["Track; 1", "First\r\nSecond \"quote\"", ""]
            ]
        );
        assert!(parse(text, ';', 1, 3).is_err());
        assert!(parse("Title\n\"truncated", ',', 3, 1).is_err());
    }
}
