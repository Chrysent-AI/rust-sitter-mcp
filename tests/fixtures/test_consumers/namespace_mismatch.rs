const LIMIT: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;

    fn check(value: usize) {
        let _: LIMIT = value;
        assert_eq!(1, 1);
    }
}
