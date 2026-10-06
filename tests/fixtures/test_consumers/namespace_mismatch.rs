const LIMIT: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;

    fn check(value: usize) {
        assert_eq!(1, 1);
        let _: LIMIT = value;
    }
}
