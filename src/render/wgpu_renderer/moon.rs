use chrono::{NaiveDate, Utc};

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
