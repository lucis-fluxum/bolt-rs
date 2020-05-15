use std::convert::TryFrom;

use chrono::{DateTime, Datelike, TimeZone, Timelike};
use chrono_tz::Tz;

use crate::error::*;
use crate::value::DateTimeZoned;
use crate::Value;

// Can't impl<T: TimeZone> From<DateTime<T>> for DateTimeZoned, since we can't get a timezone name from an Offset

impl From<DateTime<Tz>> for DateTimeZoned {
    fn from(date_time: DateTime<Tz>) -> Self {
        let zone_id = date_time.timezone().name().to_string();
        let date = date_time.date();
        let time = date_time.time();
        Self::new(
            date.year(),
            date.month(),
            date.day(),
            time.hour(),
            time.minute(),
            time.second(),
            time.nanosecond(),
            zone_id,
        )
        // If the given date_time is valid, then it's ok to unwrap
        .unwrap()
    }
}

impl TryFrom<Value> for DateTime<Tz> {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self> {
        match value {
            Value::DateTimeZoned(date_time_zoned) => {
                // Time zone guaranteed to be valid in existing objects, ok to unwrap
                let timezone: Tz = date_time_zoned.zone_id.parse().unwrap();
                Ok(timezone
                    .timestamp_opt(date_time_zoned.epoch_seconds, date_time_zoned.nanos as u32)
                    // epoch_seconds and nanos are guaranteed to be valid in existing objects, ok to unwrap
                    .unwrap())
            }
            _ => Err(ConversionError::FromValue(value).into()),
        }
    }
}

// TODO: Combine this with the conversion from Value::DateTimeOffset
// impl TryFrom<Value> for DateTime<FixedOffset> {
//     type Error = Error;
//
//     fn try_from(value: Value) -> Result<Self> {
//         match value {
//             Value::DateTimeZoned(date_time_zoned) => {
//                 // Time zone guaranteed to be valid in existing objects, ok to unwrap
//                 let timezone: Tz = date_time_zoned.zone_id.value.parse().unwrap();
//                 let timezone: FixedOffset = timezone
//                     // Get the fixed offset (e.g. Pacific Daylight vs. Pacific Standard) for the given point in time
//                     .offset_from_utc_datetime(
//                         &NaiveDateTime::from_timestamp_opt(date_time_zoned.epoch_seconds, 0)
//                             // epoch_seconds is guaranteed to be a valid timestamp, ok to unwrap
//                             .unwrap(),
//                     )
//                     .fix();
//                 Ok(timezone
//                     .timestamp_opt(date_time_zoned.epoch_seconds, date_time_zoned.nanos as u32)
//                     // epoch_seconds and nanos are guaranteed to be valid in existing objects, ok to unwrap
//                     .unwrap())
//             }
//             _ => Err(ConversionError::FromValue(value).into()),
//         }
//     }
// }
