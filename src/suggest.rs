//! Подсказка при опечатке: ближайшее допустимое имя (как `gh`: «может, rating?»).

/// Дальше опечатку уже не угадать: подсказка сбивала бы с толку.
const MAX_DISTANCE: usize = 2;

/// Ближайшее к `name` имя из `candidates`, если оно достаточно близко; при равенстве — первое.
pub fn closest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .into_iter()
        .map(|candidate| (distance(name, candidate), candidate))
        .filter(|(d, _)| *d <= MAX_DISTANCE)
        .min_by_key(|(d, _)| *d)
        .map(|(_, candidate)| candidate)
}

/// Расстояние Левенштейна по символам.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let replace = prev[j] + usize::from(ca != *cb);
            cur.push(replace.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::closest;

    #[test]
    fn suggests_only_close_names() {
        assert_eq!(closest("raiting", ["body", "rating"]), Some("rating"));
        assert_eq!(closest("xyz", ["body", "rating"]), None);
    }
}
