/// Base position grouping for read-level plots.
///
/// Replicates the logic from `Graphs/BaseGroup.java`. Early positions are shown
/// individually while later positions are grouped into bins so that general
/// trends remain visible without overwhelming the output.
/// A range of read positions (0-based, inclusive on both ends).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseGroup {
    pub lower_count: usize, // 0-based start (inclusive)
    pub upper_count: usize, // 0-based end (inclusive)
}

impl BaseGroup {
    /// Human-readable label for this group.
    ///
    /// Java's `BaseGroup.toString()` uses 1-based positions.
    /// A single-position group prints as e.g. "1", a range as "10-14".
    pub fn label(&self) -> String {
        let lower = self.lower_count + 1;
        let upper = self.upper_count + 1;
        if lower == upper {
            format!("{}", lower)
        } else {
            format!("{}-{}", lower, upper)
        }
    }

    /// Build the set of base groups for a given maximum read length.
    ///
    /// Replicates `BaseGroup.makeBaseGroups(int)`. The Java code
    /// works in 1-based coordinates internally and we convert to 0-based here.
    pub fn make_base_groups(max_length: usize, nogroup: bool, expgroup: bool) -> Vec<BaseGroup> {
        if nogroup {
            make_ungrouped_groups(max_length)
        } else if expgroup {
            make_exponential_base_groups(max_length)
        } else {
            make_linear_base_groups(max_length)
        }
    }
}

/// Replicates `makeUngroupedGroups`. Java uses 1-based coordinates;
/// we produce 0-based groups. Each position gets its own group.
fn make_ungrouped_groups(max_length: usize) -> Vec<BaseGroup> {
    (0..max_length)
        .map(|i| BaseGroup {
            lower_count: i,
            upper_count: i,
        })
        .collect()
}

/// Replicates `makeExponentialBaseGroups` exactly. The interval
/// increases at specific thresholds (positions 10, 50, 100, 500, 1000 in
/// 1-based coordinates) depending on max_length.
fn make_exponential_base_groups(max_length: usize) -> Vec<BaseGroup> {
    let mut groups = Vec::new();
    // Java works in 1-based coordinates throughout this method.
    let mut starting_base: usize = 1;
    let mut interval: usize = 1;

    while starting_base <= max_length {
        let mut end_base = starting_base + interval - 1;
        if end_base > max_length {
            end_base = max_length;
        }

        groups.push(BaseGroup {
            lower_count: starting_base - 1, // convert to 0-based
            upper_count: end_base - 1,
        });

        starting_base += interval;

        // These thresholds are checked after incrementing starting_base,
        // matching the Java code exactly.
        if starting_base == 10 && max_length > 75 {
            interval = 5;
        }
        if starting_base == 50 && max_length > 200 {
            interval = 10;
        }
        if starting_base == 100 && max_length > 300 {
            interval = 50;
        }
        if starting_base == 500 && max_length > 1000 {
            interval = 100;
        }
        if starting_base == 1000 && max_length > 2000 {
            interval = 500;
        }
    }

    groups
}

/// Replicates `getLinearInterval`. Tries intervals from the set
/// [2, 5, 10] * 10^n until the total number of groups (9 individual + grouped
/// remainder) is below 75.
fn get_linear_interval(length: usize) -> usize {
    let base_values = [2, 5, 10];
    let mut multiplier: usize = 1;

    loop {
        for &b in &base_values {
            let interval = b * multiplier;
            let mut group_count = 9 + (length - 9) / interval;
            if !(length - 9).is_multiple_of(interval) {
                group_count += 1;
            }
            if group_count < 75 {
                return interval;
            }
        }

        multiplier *= 10;

        if multiplier == 10_000_000 {
            panic!(
                "Couldn't find a sensible interval grouping for length '{}'",
                length
            );
        }
    }
}

/// Replicates `makeLinearBaseGroups`. For lengths <= 75 returns
/// ungrouped. Otherwise first 9 positions are individual, then groups of a
/// calculated interval. The special case where `starting_base == 10` and
/// `interval > 10` adjusts the first grouped bin to align to the interval
/// boundary, exactly matching the Java logic.
fn make_linear_base_groups(max_length: usize) -> Vec<BaseGroup> {
    if max_length <= 75 {
        return make_ungrouped_groups(max_length);
    }

    let interval = get_linear_interval(max_length);
    let mut groups = Vec::new();
    // Java works in 1-based coordinates.
    let mut starting_base: usize = 1;

    while starting_base <= max_length {
        let mut end_base = starting_base + interval - 1;

        // First 9 positions (1-based 1..9) are individual.
        if starting_base < 10 {
            end_base = starting_base;
        }

        // When the interval is larger than 10, the first grouped
        // bin after the individual positions extends to (interval - 1) so it
        // aligns with subsequent interval boundaries.
        if starting_base == 10 && interval > 10 {
            end_base = interval - 1;
        }

        if end_base > max_length {
            end_base = max_length;
        }

        groups.push(BaseGroup {
            lower_count: starting_base - 1,
            upper_count: end_base - 1,
        });

        if starting_base < 10 {
            starting_base += 1;
        } else if starting_base == 10 && interval > 10 {
            // Jump to the interval boundary.
            starting_base = interval;
        } else {
            starting_base += interval;
        }
    }

    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_label_single() {
        let g = BaseGroup {
            lower_count: 0,
            upper_count: 0,
        };
        assert_eq!(g.label(), "1");
    }

    #[test]
    fn test_label_range() {
        let g = BaseGroup {
            lower_count: 9,
            upper_count: 13,
        };
        assert_eq!(g.label(), "10-14");
    }

    #[test]
    fn test_ungrouped_10() {
        let groups = BaseGroup::make_base_groups(10, true, false);
        assert_eq!(groups.len(), 10);
        assert_eq!(groups[0].label(), "1");
        assert_eq!(groups[9].label(), "10");
    }

    #[test]
    fn test_linear_short_ungrouped() {
        // <= 75 should be ungrouped even without nogroup flag
        let groups = BaseGroup::make_base_groups(50, false, false);
        assert_eq!(groups.len(), 50);
    }

    #[test]
    fn test_linear_100() {
        let groups = BaseGroup::make_base_groups(100, false, false);
        // Should have 9 individual + grouped remainder
        assert!(groups.len() < 75);
        // First 9 are individual
        for (i, group) in groups.iter().enumerate().take(9) {
            assert_eq!(group.lower_count, i);
            assert_eq!(group.upper_count, i);
        }
    }

    #[test]
    fn test_exponential_150() {
        let groups = BaseGroup::make_base_groups(150, false, true);
        // First 9 individual, then groups of 5 (since 150 > 75)
        assert_eq!(groups[0].label(), "1");
        assert_eq!(groups[8].label(), "9");
        // Position 10 starts grouped by 5: 10-14
        assert_eq!(groups[9].label(), "10-14");
    }

    #[test]
    fn test_exponential_250() {
        let groups = BaseGroup::make_base_groups(250, false, true);
        // After position 50 (1-based), interval goes to 10 since 250 > 200
        // Find the group starting at position 50 (1-based)
        let g50 = groups.iter().find(|g| g.lower_count == 49).unwrap();
        assert_eq!(g50.label(), "50-59");
    }

    #[test]
    fn test_get_linear_interval_100() {
        let interval = get_linear_interval(100);
        assert_eq!(interval, 2);
    }

    #[test]
    fn test_get_linear_interval_300() {
        let interval = get_linear_interval(300);
        let group_count = 9 + (300 - 9) / interval + if (300 - 9) % interval != 0 { 1 } else { 0 };
        assert!(group_count < 75);
    }
}
