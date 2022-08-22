use std::{time::Instant, mem::swap};
use modality_ingest_client::types::TimelineId;

/// A Least Recently Used (LRU) cache of timeline information
pub struct TimelineLru {
    // TODO(AJM): At the moment, the contents of the LRU cache are unsorted.
    //
    // Sorting them by time would be BAD, because there would be lots of churn.
    // However, sorting them by `user_id` might be good, because then we could
    // use a binary search for the fast path of query, but then we'd need to
    // still do a linear search to determine the oldest item.
    //
    // For now, a linear search of a relatively small number (64-128) of items
    // is likely to be more than fast enough. If it later turns out that users
    // need a much larger LRU cache size (e.g. 1024+), we might consider making this
    // optimization at that time.
    data: Vec<LruItem>,
}

impl TimelineLru {
    /// Create a timeline LRU cache with a maximum size of `cap`.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            data: Vec::with_capacity(cap),
        }
    }

    /// Check to see if the given `user_id` is known by the Lru
    pub fn query(&mut self, user_id: u64) -> Result<TimelineId, LruToken> {
        let full = self.data.len() >= self.data.capacity();

        if !full {
            // If we're not full, we don't need to keep track of the oldest item.
            // Just find it or don't.
            match self.data.iter_mut().find(|li| li.user_id == user_id) {
                Some(li) => {
                    li.last_use = Instant::now();
                    Ok(li.timeline_id)
                },
                None => Err(LruToken { val: LruError::DoesntExistNotFull }),
            }
        } else {
            // Traverse the list, remembering the oldest slot we've seen. We'll either:
            //
            // * Find the item, bailing out early
            // * Reach the end, in which case we've already found the oldest item
            let mut oldest_time = Instant::now();
            let mut oldest_slot = 0;
            for (idx, li) in self.data.iter_mut().enumerate() {
                if li.user_id == user_id {
                    li.last_use = Instant::now();
                    return Ok(li.timeline_id);
                }

                if oldest_time >= li.last_use {
                    oldest_time = li.last_use;
                    oldest_slot = idx;
                }
            }

            // We reached the end, it doesn't exist.
            Err(LruToken { val: LruError::DoesntExistReplace(oldest_slot) })
        }
    }

    pub fn insert(&mut self, user_id: u64, timeline_id: TimelineId, token: LruToken) {
        let mut item = LruItem {
            user_id,
            timeline_id,
            last_use: Instant::now(),
        };

        match token.val {
            LruError::DoesntExistNotFull => self.data.push(item),
            LruError::DoesntExistReplace(idx) => {
                if let Some(old) = self.data.get_mut(idx) {
                    swap(old, &mut item)
                }
            }
        }
    }
}

struct LruItem {
    user_id: u64,
    timeline_id: TimelineId,
    last_use: Instant,
}

pub struct LruToken {
    // Note: This is a struct with a private field to ensure that
    // `query` must be called before `insert`.
    val: LruError
}

enum LruError {
    DoesntExistNotFull,
    DoesntExistReplace(usize),
}
