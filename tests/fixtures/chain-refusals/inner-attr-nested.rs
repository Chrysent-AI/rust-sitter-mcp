fn selected() {}
fn retained() {
    match () {
        #![cfg_attr(feature = "strict", no_implicit_prelude)]
        () => {}
    }
}
