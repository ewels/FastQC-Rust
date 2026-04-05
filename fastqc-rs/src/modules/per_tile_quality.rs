// Per Tile Sequence Quality module
// Corresponds to Modules/PerTileQualityScores.java

use std::collections::HashMap;
use std::io;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::tile_graph::{TileGraphData, render_tile_graph};
use crate::sequence::Sequence;
use crate::utils::base_group::BaseGroup;
use crate::utils::format::java_format_double;
use crate::utils::{phred, quality_count};
use crate::utils::quality_count::QualityCount;

pub struct PerTileQualityScores {
    per_tile_quality_counts: HashMap<i32, Vec<QualityCount>>,
    current_length: usize,
    total_count: u64,
    // splitPosition tracks which colon-separated field contains the tile number.
    // -1 means not yet determined.
    split_position: i32,
    ignore_in_report: bool,
    nogroup: bool,
    expgroup: bool,
    limits: Limits,
}

impl PerTileQualityScores {
    pub fn new(limits: &Limits, nogroup: bool, expgroup: bool) -> Self {
        PerTileQualityScores {
            per_tile_quality_counts: HashMap::new(),
            current_length: 0,
            total_count: 0,
            split_position: -1,
            ignore_in_report: false,
            nogroup,
            expgroup,
            limits: limits.clone(),
        }
    }

    fn calculate(&self) -> Option<TileCalculatedData> {
        if self.per_tile_quality_counts.is_empty() {
            return None;
        }

        // Collect all QualityCount slices across tiles to find global min/max chars
        let all_counts: Vec<&QualityCount> = self.per_tile_quality_counts
            .values()
            .flat_map(|v| v.iter())
            .collect();
        let (min_char, _max_char) = quality_count::calculate_offsets(all_counts);
        // If no quality data, default to Sanger offset (33).
        let offset = phred::detect(min_char)
            .map(|e| e.offset)
            .unwrap_or(33);

        let groups = BaseGroup::make_base_groups(
            self.current_length,
            self.nogroup,
            self.expgroup,
        );

        let mut tile_numbers: Vec<i32> = self.per_tile_quality_counts.keys().copied().collect();
        tile_numbers.sort();

        let mut means = vec![vec![0.0f64; groups.len()]; tile_numbers.len()];
        let mut x_labels = Vec::with_capacity(groups.len());

        for (t, &tile) in tile_numbers.iter().enumerate() {
            for (i, group) in groups.iter().enumerate() {
                if t == 0 {
                    x_labels.push(group.label());
                }
                let min_base = group.lower_count;
                let max_base = group.upper_count;
                means[t][i] = self.get_mean(tile, min_base, max_base, offset);
            }
        }

        // Normalise by subtracting column averages to show deviations
        let mut average_qualities_per_group = vec![0.0f64; groups.len()];
        for tile_means in means.iter().take(tile_numbers.len()) {
            for (avg, &m) in average_qualities_per_group.iter_mut().zip(tile_means.iter()) {
                *avg += m;
            }
        }
        for avg in &mut average_qualities_per_group {
            *avg /= tile_numbers.len() as f64;
        }

        let mut max_deviation: f64 = 0.0;
        // subtract per-group averages from each tile's means
        for (i, &avg) in average_qualities_per_group.iter().enumerate() {
            for tile_means in means.iter_mut().take(tile_numbers.len()) {
                tile_means[i] -= avg;
                if tile_means[i].abs() > max_deviation {
                    max_deviation = tile_means[i].abs();
                }
            }
        }

        Some(TileCalculatedData {
            tiles: tile_numbers,
            means,
            x_labels,
            max_deviation,
        })
    }

    /// Replicates `getMean(int tile, int minbp, int maxbp, int offset)`.
    fn get_mean(&self, tile: i32, min_base: usize, max_base: usize, offset: u8) -> f64 {
        let quality_counts = match self.per_tile_quality_counts.get(&tile) {
            Some(qc) => qc,
            None => return 0.0,
        };

        let mut count = 0;
        let mut total = 0.0;

        for qc in &quality_counts[min_base..=max_base] {
            if qc.get_total_count() > 0 {
                count += 1;
                total += qc.get_mean(offset);
            }
        }

        if count > 0 {
            total / count as f64
        } else {
            0.0
        }
    }
}

impl PerTileQualityScores {
    fn build_chart_svg(&self) -> Option<String> {
        let data = self.calculate()?;

        // Color scale max is the error threshold from config
        let color_scale_max = self.limits.threshold("tile\terror", 5.0);

        Some(render_tile_graph(&TileGraphData {
            x_labels: data.x_labels,
            tiles: data.tiles,
            tile_base_means: data.means,
            color_scale_max,
        }))
    }
}

impl QCModule for PerTileQualityScores {
    fn process_sequence(&mut self, sequence: &Sequence) {
        // Check ignore config on first sequence
        if self.total_count == 0 && self.limits.is_ignored("tile") {
            self.ignore_in_report = true;
        }

        if self.ignore_in_report {
            return;
        }

        // Skip zero-length quality strings
        if sequence.quality.is_empty() {
            return;
        }

        self.total_count += 1;

        // Sample all for first 10k reads, then every 10th
        if self.total_count > 10000 && !self.total_count.is_multiple_of(10) {
            return;
        }

        // Parse tile ID from read header.
        // Use nth() on the split iterator to avoid allocating a Vec per sequence.
        if self.split_position < 0 {
            let field_count = sequence.id.split(':').count();
            if field_count >= 7 {
                // 1.8+ format, tile at position 4
                self.split_position = 4;
            } else if field_count >= 5 {
                // Older format, tile at position 2
                self.split_position = 2;
            } else {
                // Can't get a tile from this header
                self.ignore_in_report = true;
                return;
            }
        }

        let tile = match sequence.id.split(':').nth(self.split_position as usize)
            .and_then(|f| f.parse::<i32>().ok())
        {
            Some(t) => t,
            None => {
                self.ignore_in_report = true;
                return;
            }
        };

        let qual = &sequence.quality;

        // Grow all existing tile arrays if quality string is longer
        if self.current_length < qual.len() {
            for qc_vec in self.per_tile_quality_counts.values_mut() {
                qc_vec.resize_with(qual.len(), QualityCount::new);
            }
            self.current_length = qual.len();
        }

        // Add new tile if not seen, with check for too many tiles
        if !self.per_tile_quality_counts.contains_key(&tile) {
            if self.per_tile_quality_counts.len() > 2500 {
                // Too many tiles, give up
                eprintln!("Too many tiles (>2500) so giving up trying to do per-tile qualities since we're probably parsing the file wrongly");
                self.ignore_in_report = true;
                self.per_tile_quality_counts.clear();
                return;
            }

            let mut quality_counts = Vec::new();
            quality_counts.resize_with(self.current_length, QualityCount::new);
            self.per_tile_quality_counts.insert(tile, quality_counts);
        }

        let quality_counts = self.per_tile_quality_counts.get_mut(&tile).unwrap();

        for (i, &q) in qual.iter().enumerate() {
            quality_counts[i].add_value(q);
        }
    }

    fn name(&self) -> &str {
        "Per tile sequence quality"
    }

    fn description(&self) -> &str {
        "Shows the per tile Quality scores of all bases at a given position in a sequencing run"
    }

    fn reset(&mut self) {
        self.total_count = 0;
        self.per_tile_quality_counts.clear();
        self.current_length = 0;
        self.split_position = -1;
        self.ignore_in_report = false;
    }

    fn raises_error(&self) -> bool {
        let threshold = self.limits.threshold("tile\terror", 5.0);
        self.calculate().is_some_and(|data| data.max_deviation > threshold)
    }

    fn raises_warning(&self) -> bool {
        let threshold = self.limits.threshold("tile\twarn", 2.0);
        self.calculate().is_some_and(|data| data.max_deviation > threshold)
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        // Ignore if flagged, configured to ignore, or no data
        self.ignore_in_report
            || self.limits.is_ignored("tile")
            || self.current_length == 0
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let data = match self.calculate() {
            Some(d) => d,
            None => return Ok(()),
        };

        // Header and format match Java's makeReport
        writeln!(writer, "#Tile\tBase\tMean")?;

        for (t, &tile) in data.tiles.iter().enumerate() {
            for i in 0..data.means[t].len() {
                writeln!(
                    writer,
                    "{}\t{}\t{}",
                    tile,
                    data.x_labels[i],
                    java_format_double(data.means[t][i]),
                )?;
            }
        }

        Ok(())
    }

    // Image filename matches Java's "per_tile_quality.png" in Images/
    fn chart_image_name(&self) -> Option<&str> { Some("per_tile_quality") }
    fn chart_alt_text(&self) -> Option<&str> { Some("Per tile sequence quality") }
    fn generate_chart_svg(&self) -> Option<String> { self.build_chart_svg() }
}

struct TileCalculatedData {
    tiles: Vec<i32>,
    means: Vec<Vec<f64>>,
    x_labels: Vec<String>,
    max_deviation: f64,
}
