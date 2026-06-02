use chrono::{NaiveDate, Utc};

/// Phase used for hub moon shading and related globals (`0` = new, `0.5` = full).
pub(crate) fn main_menu_moon_phase_for_render(use_live_calendar: bool, forced_phase: f32) -> f32 {
    if use_live_calendar {
        current_moon_phase()
    } else {
        forced_phase.clamp(0.0, 1.0)
    }
}

/// Short label for debug overlay readouts (approximate by synodic fraction).
pub fn moon_phase_short_name(phase: f32) -> &'static str {
    let p = phase.clamp(0.0, 1.0);
    if p < 0.06 || p > 0.94 {
        "New moon"
    } else if (p - 0.25).abs() < 0.06 {
        "First quarter"
    } else if (p - 0.5).abs() < 0.06 {
        "Full moon"
    } else if (p - 0.75).abs() < 0.06 {
        "Last quarter"
    } else if p < 0.25 {
        "Waxing crescent"
    } else if p < 0.5 {
        "Waxing gibbous"
    } else if p < 0.75 {
        "Waning gibbous"
    } else {
        "Waning crescent"
    }
}

pub(crate) fn current_moon_phase() -> f32 {
    let known_new_moon = NaiveDate::from_ymd_opt(2000, 1, 6)
        .expect("valid new moon reference date")
        .and_hms_opt(18, 14, 0)
        .expect("valid new moon reference time");
    let days_since_reference =
        (Utc::now().naive_utc() - known_new_moon).num_seconds() as f64 / 86_400.0;
    let synodic_month_days = 29.530_588_853_f64;
    (days_since_reference.rem_euclid(synodic_month_days) / synodic_month_days) as f32
}
