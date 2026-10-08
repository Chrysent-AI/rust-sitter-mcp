use crate::{AllowHeader, MethodRouter};

fn first_route() -> u8 { 0 }
fn second_route() -> u8 { 1 }

impl AllowHeader {
    fn merge(self, _other: Self) -> Self {
        #[cfg(debug_assertions)]
        assert!(true);
        self
    }
}

impl<A> MethodRouter<A>
where
    A: Clone,
{
    fn identity(value: A) -> A { value }
    fn on<H>(&self, handler: H) -> H { handler }

    #[cfg(debug_assertions)]
    fn debug_route(&self) -> u8 { 0 }
}

impl<A> MethodRouter<A> {
    fn count(&self) -> u8 { 1 }
}

impl<A> crate::Route for MethodRouter<A> {
    fn route(&self) -> u8 { 2 }
}

#[cfg(feature = "enabled")]
impl<A> MethodRouter<A> {
    fn conditional_route(&self) -> u8 { 3 }
}
