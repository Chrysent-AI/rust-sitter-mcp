fn selected() {}
fn retained() {
    match () {
        #![cfg_attr(feature = "strict", deny(warnings))]
        () => {}
    }
}
