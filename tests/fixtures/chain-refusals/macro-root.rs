macro_rules! crate_root {
    () => {
        mod branch;
        include!(concat!(env!("OUT_DIR"), "/generated.rs"));
    };
}
crate_root!();
