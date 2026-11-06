//! Audio effects: volume normalization, the equalizer and channel selection.
//! They all end up in one mpv `af` filter graph.

/// Equalizer band centers in Hz.
pub const BANDS: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
/// Band gains go from -MAX_GAIN to +MAX_GAIN dB.
pub const MAX_GAIN: i32 = 12;

pub type Gains = [i32; BANDS.len()];

pub const FLAT: Gains = [0; BANDS.len()];

pub const PRESETS: [(&str, Gains); 5] = [
    ("关闭", FLAT),
    ("低音", [6, 5, 4, 2, 0, 0, 0, 0, 0, 0]),
    ("人声", [-2, -2, -1, 1, 3, 4, 4, 2, 0, -1]),
    ("高音", [0, 0, 0, 0, 0, 1, 2, 4, 5, 6]),
    ("影院", [4, 3, 1, 0, -1, 0, 1, 2, 3, 3]),
];

/// RMS-based, because peak-based settings barely closed the gap between loud
/// and quiet passages when measured on a test signal.
const NORMALIZER: &str = "dynaudnorm=f=200:g=11:p=0.8:m=16:r=0.15";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Channel {
    #[default]
    Stereo,
    Left,
    Right,
    /// Both sides mixed together, for files with one silent side.
    Mixed,
}

impl Channel {
    /// Downmixes to stereo first, so mono and 5.1 sources work as well.
    fn filter(self) -> Option<&'static str> {
        Some(match self {
            Self::Stereo => return None,
            Self::Left => "aformat=channel_layouts=stereo,pan=stereo|c0=c0|c1=c0",
            Self::Right => "aformat=channel_layouts=stereo,pan=stereo|c0=c1|c1=c1",
            Self::Mixed => "aformat=channel_layouts=stereo,pan=stereo|c0=0.5*c0+0.5*c1|c1=0.5*c0+0.5*c1",
        })
    }
}

pub fn encode_gains(g: &Gains) -> String {
    g.map(|v| v.to_string()).join(",")
}

/// Bad or missing values give a flat equalizer rather than a surprise.
pub fn parse_gains(s: &str) -> Gains {
    let parsed: Option<Vec<i32>> = s.split(',').map(|v| v.parse().ok()).collect();
    match parsed.and_then(|v| Gains::try_from(v).ok()) {
        Some(g) => g.map(|v| v.clamp(-MAX_GAIN, MAX_GAIN)),
        None => FLAT,
    }
}

/// The value for mpv's `af` property; empty when nothing is on.
pub fn filter_graph(normalize: bool, gains: &Gains, channel: Channel) -> String {
    let mut parts: Vec<String> = channel.filter().map(str::to_string).into_iter().collect();
    // Lower the signal by the biggest boost so the boosted band can't clip.
    let boost = gains.iter().copied().max().unwrap_or(0);
    if boost > 0 {
        parts.push(format!("volume=-{boost}dB"));
    }
    parts.extend(
        BANDS.iter().zip(gains).filter(|(_, g)| **g != 0).map(|(f, g)| format!("equalizer=f={f}:t=o:w=1:g={g}")),
    );
    if normalize {
        parts.push(NORMALIZER.to_string());
    }
    if parts.is_empty() { String::new() } else { format!("lavfi=[{}]", parts.join(",")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_on_means_no_filter() {
        assert_eq!(filter_graph(false, &FLAT, Channel::Stereo), "");
    }

    #[test]
    fn filters_run_channel_then_eq_then_normalizer() {
        let mut g = FLAT;
        g[5] = 6;
        g[8] = -3;
        let graph = filter_graph(true, &g, Channel::Left);
        let (ch, eq, norm) = (graph.find("pan=").unwrap(), graph.find("equalizer").unwrap(), graph.find("dynaudnorm").unwrap());
        assert!(ch < eq && eq < norm);
        assert!(graph.contains("volume=-6dB,equalizer=f=1000:t=o:w=1:g=6,equalizer=f=8000:t=o:w=1:g=-3"));
    }

    #[test]
    fn cuts_alone_need_no_headroom() {
        let mut g = FLAT;
        g[0] = -4;
        assert!(!filter_graph(false, &g, Channel::Stereo).contains("volume"));
    }

    #[test]
    fn gains_round_trip_and_bad_input_is_flat() {
        let g = PRESETS[2].1;
        assert_eq!(parse_gains(&encode_gains(&g)), g);
        assert_eq!(parse_gains(""), FLAT);
        assert_eq!(parse_gains("1,2,3"), FLAT);
        assert_eq!(parse_gains("99,0,0,0,0,0,0,0,0,-99")[0], MAX_GAIN);
    }
}
