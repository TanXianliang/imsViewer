pub fn calibrated_confidence(similarity: f32, exact: bool) -> f32 {
    if exact {
        return 100.0;
    }
    let points = [
        (0.45f32, 5.0f32),
        (0.55, 20.0),
        (0.65, 55.0),
        (0.75, 90.0),
        (0.85, 97.0),
        (1.0, 99.9),
    ];
    let value = similarity.clamp(points[0].0, points[points.len() - 1].0);
    for window in points.windows(2) {
        let (left_x, left_y) = window[0];
        let (right_x, right_y) = window[1];
        if value <= right_x {
            let fraction = (value - left_x) / (right_x - left_x);
            return left_y + fraction * (right_y - left_y);
        }
    }
    99.9
}

pub fn is_template_risk(similarity: f32, phash_distance: u32) -> bool {
    similarity >= 0.85 && phash_distance >= 18
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_is_monotonic() {
        assert!(calibrated_confidence(0.85, false) > calibrated_confidence(0.75, false));
        assert_eq!(calibrated_confidence(0.5, true), 100.0);
    }
}
