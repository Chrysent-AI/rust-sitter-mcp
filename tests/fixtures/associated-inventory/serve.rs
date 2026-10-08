use crate::{ConnectionLifetimeLimits, MaxConnectionAge, Serve};

fn default_age() -> u8 { 0 }
fn bounded_age() -> u8 { 1 }

impl ConnectionLifetimeLimits {
    /// Apply a connection age limit.
    pub fn max_connection_age(mut self, age: MaxConnectionAge) -> Self {
        self.age = age.age;
        self
    }

    fn identity(age: u8) -> u8 { age }
    const UNLIMITED: u8 = 0;
}

impl MaxConnectionAge {
    fn age(self) -> u8 { self.age }
}

impl<'a, L> Serve<'a, L>
where
    L: Clone,
{
    fn local_addr(&self) -> u8 { 0 }

    #[cfg(debug_assertions)]
    fn debug_addr(&self) -> u8 { 1 }
}

impl ConnectionLifetimeLimits {
    fn state(&self) -> u8 { self.age }
}
