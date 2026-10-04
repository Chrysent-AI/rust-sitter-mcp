use crate::result::DomainError;
use std::{
    collections::{BTreeMap, hash_map::RandomState},
    hash::BuildHasher,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Series {
    pub request: String,
    pub snapshot: String,
    created: Instant,
}
#[derive(Default)]
pub struct Cursors {
    records: BTreeMap<u64, Series>,
    key: RandomState,
    next: u64,
}
impl Cursors {
    pub fn lookup(
        &self,
        token: &str,
        request: &str,
    ) -> Result<(u64, Series, usize, usize), DomainError> {
        let invalid = || {
            DomainError::new(
                "INVALID_CURSOR",
                "invalid or unknown cursor; restart search without it",
            )
        };
        if token.len() > 200 || !token.is_ascii() {
            return Err(invalid());
        }
        let parts: Vec<_> = token.split('/').collect();
        if parts.len() != 5 || parts[0] != "v1" {
            return Err(invalid());
        }
        let id = parts[1].parse::<u64>().map_err(|_| invalid())?;
        let file = parts[2].parse::<usize>().map_err(|_| invalid())?;
        let position = parts[3].parse::<usize>().map_err(|_| invalid())?;
        if self.token(id, file, position) != token {
            return Err(invalid());
        }
        let series = self.records.get(&id).ok_or_else(invalid)?;
        if series.created.elapsed() >= Duration::from_secs(900) {
            return Err(DomainError::new(
                "CURSOR_EXPIRED",
                "cursor series expired after 15 minutes",
            ));
        }
        if series.request != request {
            return Err(DomainError::new(
                "CURSOR_SCOPE_MISMATCH",
                "repeat the identical effective request and root",
            ));
        }
        Ok((id, series.clone(), file, position))
    }
    pub fn insert(&mut self, request: String, snapshot: String) -> Result<u64, DomainError> {
        self.records
            .retain(|_, series| series.created.elapsed() < Duration::from_secs(900));
        if self.records.len() >= 32 {
            return Err(DomainError::new(
                "CURSOR_CAPACITY",
                "32 active pagination series; retry after expiry",
            ));
        }
        self.next = self.next.checked_add(1).ok_or_else(|| {
            DomainError::new(
                "CURSOR_CAPACITY",
                "series counter exhausted; restart server",
            )
        })?;
        self.records.insert(
            self.next,
            Series {
                request,
                snapshot,
                created: Instant::now(),
            },
        );
        Ok(self.next)
    }
    pub fn token(&self, id: u64, file: usize, position: usize) -> String {
        let fields = format!("v1/{id}/{file}/{position}");
        format!("{fields}/{:016x}", self.key.hash_one(&fields))
    }
}
