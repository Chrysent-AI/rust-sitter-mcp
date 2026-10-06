macro_rules! crate_root {
    () => {
        emit!([({ mod branch {} })]);
    };
}
crate_root!();
