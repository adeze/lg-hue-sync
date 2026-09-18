/// Official Philips Hue color gamuts and chromaticity conversions
/// ported from HueEntertainmentKit (Swift) to native Rust.

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum HueGamut {
    /// LivingColors, Bloom, Aura, Iris
    GamutA,
    /// Hue bulbs Gen 1 and 2, Lightstrips Gen 1, Spotlights
    GamutB,
    /// Hue White and Color Ambiance Gen 3+, Lightstrip Plus, Play Bars, Signe, Iris Gen 4
    GamutC,
}

impl Default for HueGamut {
    fn default() -> Self {
        HueGamut::GamutC
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HueXYBrightness {
    pub x: f64,
    pub y: f64,
    pub brightness: f64,
}

impl HueGamut {
    pub fn red(&self) -> Point {
        match self {
            HueGamut::GamutA => Point::new(0.704, 0.296),
            HueGamut::GamutB => Point::new(0.675, 0.322),
            HueGamut::GamutC => Point::new(0.6915, 0.3083),
        }
    }

    pub fn green(&self) -> Point {
        match self {
            HueGamut::GamutA => Point::new(0.2151, 0.7106),
            HueGamut::GamutB => Point::new(0.409, 0.518),
            HueGamut::GamutC => Point::new(0.17, 0.7),
        }
    }

    pub fn blue(&self) -> Point {
        match self {
            HueGamut::GamutA => Point::new(0.138, 0.08),
            HueGamut::GamutB => Point::new(0.167, 0.04),
            HueGamut::GamutC => Point::new(0.1532, 0.0475),
        }
    }

    /// Determines whether the given chromaticity point lies within this gamut triangle.
    pub fn contains(&self, p: Point) -> bool {
        let r = self.red();
        let g = self.green();
        let b = self.blue();

        fn cross(a: Point, b: Point, c: Point) -> f64 {
            (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
        }

        let cp1 = cross(r, g, p);
        let cp2 = cross(g, b, p);
        let cp3 = cross(b, r, p);

        let has_neg = (cp1 < 0.0) || (cp2 < 0.0) || (cp3 < 0.0);
        let has_pos = (cp1 > 0.0) || (cp2 > 0.0) || (cp3 > 0.0);

        !(has_neg && has_pos)
    }

    /// Clamps the given point to the nearest boundary of this gamut triangle if it falls outside.
    pub fn clamp(&self, point: Point) -> Point {
        if self.contains(point) {
            return point;
        }

        let r = self.red();
        let g = self.green();
        let b = self.blue();

        let p1 = closest_point_on_segment(r, g, point);
        let p2 = closest_point_on_segment(g, b, point);
        let p3 = closest_point_on_segment(b, r, point);

        let d1 = distance_squared(point, p1);
        let d2 = distance_squared(point, p2);
        let d3 = distance_squared(point, p3);

        if d1 <= d2 && d1 <= d3 {
            p1
        } else if d2 <= d1 && d2 <= d3 {
            p2
        } else {
            p3
        }
    }
}

fn closest_point_on_segment(a: Point, b: Point, p: Point) -> Point {
    let ab_x = b.x - a.x;
    let ab_y = b.y - a.y;
    let ap_x = p.x - a.x;
    let ap_y = p.y - a.y;

    let ab_len_sq = ab_x * ab_x + ab_y * ab_y;
    if ab_len_sq <= 0.0 {
        return a;
    }

    let t = ((ap_x * ab_x + ap_y * ab_y) / ab_len_sq).clamp(0.0, 1.0);
    Point::new(a.x + t * ab_x, a.y + t * ab_y)
}

fn distance_squared(a: Point, b: Point) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

/// Converts sRGB (0..255) to CIE 1931 xy coordinates and brightness, clamped to Hue Gamut C.
pub fn rgb_to_xy_brightness(r: u8, g: u8, b: u8, gamut: HueGamut) -> HueXYBrightness {
    let r_norm = r as f64 / 255.0;
    let g_norm = g as f64 / 255.0;
    let b_norm = b as f64 / 255.0;

    // 1. Gamma correction (sRGB to linear RGB)
    let r_lin = if r_norm > 0.04045 {
        ((r_norm + 0.055) / 1.055).powf(2.4)
    } else {
        r_norm / 12.92
    };

    let g_lin = if g_norm > 0.04045 {
        ((g_norm + 0.055) / 1.055).powf(2.4)
    } else {
        g_norm / 12.92
    };

    let b_lin = if b_norm > 0.04045 {
        ((b_norm + 0.055) / 1.055).powf(2.4)
    } else {
        b_norm / 12.92
    };

    // 2. Linear RGB to XYZ (Wide RGB D65 conversion matrix recommended by Philips Hue)
    let x = r_lin * 0.664511 + g_lin * 0.154324 + b_lin * 0.162028;
    let y = r_lin * 0.283881 + g_lin * 0.668433 + b_lin * 0.047685;
    let z = r_lin * 0.000088 + g_lin * 0.072310 + b_lin * 0.986039;

    let sum = x + y + z;
    let brightness = y.clamp(0.0, 1.0);

    if sum <= 0.0 {
        let clamped = gamut.clamp(gamut.red());
        return HueXYBrightness {
            x: clamped.x,
            y: clamped.y,
            brightness: 0.0,
        };
    }

    let raw_point = Point::new(x / sum, y / sum);
    let clamped = gamut.clamp(raw_point);

    HueXYBrightness {
        x: clamped.x.clamp(0.0, 1.0),
        y: clamped.y.clamp(0.0, 1.0),
        brightness,
    }
}
