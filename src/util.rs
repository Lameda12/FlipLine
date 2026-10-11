use std::cmp::Ordering;

/// Case-insensitive "natural" ordering: `frame2` < `frame10`, `a1b2` < `a1b10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(&c) = ai.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(&c) = bi.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    nb.push(c);
                    bi.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
                // Same value: fewer leading zeros first, for a stable total order.
                let ord = na.len().cmp(&nb.len());
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let (lx, ly) = (x.to_lowercase().next().unwrap_or(x), y.to_lowercase().next().unwrap_or(y));
                if lx != ly {
                    return lx.cmp(&ly);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut v: Vec<&str>) -> Vec<&str> {
        v.sort_by(|a, b| natural_cmp(a, b));
        v
    }

    #[test]
    fn numbers_sort_by_value_not_lexically() {
        assert_eq!(
            sorted(vec!["frame10.png", "frame2.png", "frame1.png", "frame11.png"]),
            ["frame1.png", "frame2.png", "frame10.png", "frame11.png"]
        );
    }

    #[test]
    fn handles_padding_case_and_mixed_segments() {
        assert_eq!(sorted(vec!["b_002", "B_001", "a_10", "a_9"]), ["a_9", "a_10", "B_001", "b_002"]);
        assert_eq!(sorted(vec!["x0010", "x010", "x10"]).len(), 3);
        assert_eq!(natural_cmp("img", "img1"), Ordering::Less);
        assert_eq!(natural_cmp("a", "a"), Ordering::Equal);
    }
}
