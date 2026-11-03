const MAX_JUMPS: usize = 100;
const SAME_SPOT: f32 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub page: usize,
    pub within: f32,
    pub across: f32,
}

impl Place {
    fn same_spot(&self, other: &Place) -> bool {
        self.page == other.page && (self.within - other.within).abs() < SAME_SPOT
    }
}

#[derive(Default)]
pub struct Jumps {
    places: Vec<Place>,
    index: usize,
}

impl Jumps {
    pub fn record(&mut self, place: Place) {
        self.places.retain(|kept| !kept.same_spot(&place));
        self.places.push(place);
        if self.places.len() > MAX_JUMPS {
            self.places.remove(0);
        }
        self.index = self.places.len();
    }

    pub fn back(&mut self, current: Place) -> Option<Place> {
        if self.places.is_empty() {
            return None;
        }
        if self.index == self.places.len() {
            self.record(current);
            self.index = self.places.len() - 1;
        }
        self.index = self.index.checked_sub(1)?;
        Some(self.places[self.index])
    }

    pub fn forward(&mut self) -> Option<Place> {
        let next = self.index + 1;
        let place = *self.places.get(next)?;
        self.index = next;
        Some(place)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(page: usize) -> Place {
        Place { page, within: 0.0, across: 0.5 }
    }

    #[test]
    fn back_and_forward_walk_the_list_and_stop_at_the_ends() {
        let mut jumps = Jumps::default();
        jumps.record(at(1));
        jumps.record(at(50));
        jumps.record(at(100));
        assert_eq!(jumps.back(at(150)), Some(at(100)));
        assert_eq!(jumps.back(at(100)), Some(at(50)));
        assert_eq!(jumps.back(at(50)), Some(at(1)));
        assert_eq!(jumps.back(at(1)), None);
        assert_eq!(jumps.forward(), Some(at(50)));
        assert_eq!(jumps.forward(), Some(at(100)));
        assert_eq!(jumps.forward(), Some(at(150)));
        assert_eq!(jumps.forward(), None);
    }

    #[test]
    fn nothing_recorded_means_nowhere_to_go() {
        let mut jumps = Jumps::default();
        assert_eq!(jumps.back(at(5)), None);
        assert_eq!(jumps.forward(), None);
        assert!(jumps.places.is_empty());
    }

    #[test]
    fn jumping_from_the_middle_keeps_older_places_and_moves_this_one_to_the_end() {
        let mut jumps = Jumps::default();
        jumps.record(at(1));
        jumps.record(at(50));
        assert_eq!(jumps.back(at(100)), Some(at(50)));
        jumps.record(at(50));
        assert_eq!(jumps.back(at(200)), Some(at(50)));
        assert_eq!(jumps.back(at(50)), Some(at(100)));
        assert_eq!(jumps.back(at(100)), Some(at(1)));
    }

    #[test]
    fn the_same_spot_is_only_kept_once() {
        let mut jumps = Jumps::default();
        jumps.record(at(1));
        jumps.record(Place { page: 1, within: 0.005, across: 0.1 });
        assert_eq!(jumps.places.len(), 1);
        jumps.record(Place { page: 1, within: 0.5, across: 0.5 });
        assert_eq!(jumps.places.len(), 2);
    }

    #[test]
    fn going_back_from_a_recorded_spot_skips_past_it() {
        let mut jumps = Jumps::default();
        jumps.record(at(1));
        jumps.record(at(2));
        assert_eq!(jumps.back(at(2)), Some(at(1)));
    }

    #[test]
    fn the_list_is_capped_by_dropping_the_oldest() {
        let mut jumps = Jumps::default();
        for page in 0..MAX_JUMPS + 5 {
            jumps.record(at(page));
        }
        assert_eq!(jumps.places.len(), MAX_JUMPS);
        assert_eq!(jumps.places[0], at(5));
    }
}
