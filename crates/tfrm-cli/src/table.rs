//! Minimal left-aligned table rendering; no external table crate.

pub fn render(headers: &[&str], rows: &[Vec<String>]) -> String {
    let cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(cols) {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let render_row = |cells: Vec<&str>| -> String {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            if i + 1 == cols {
                line.push_str(cell); // no trailing padding
            } else {
                line.push_str(&format!("{cell:<width$}", width = widths[i]));
            }
        }
        line.trim_end().to_string()
    };
    out.push_str(&render_row(headers.to_vec()));
    out.push('\n');
    for row in rows {
        out.push_str(&render_row(row.iter().map(String::as_str).collect()));
        out.push('\n');
    }
    out
}
