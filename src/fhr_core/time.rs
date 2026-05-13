pub fn parse_monitor_timestamp(value: &str) -> Result<i64, String> {
    let normalized = value.trim().trim_end_matches('Z').replace('T', " ");
    let (date, time) = normalized
        .split_once(' ')
        .ok_or_else(|| format!("timestamp is missing date/time separator: {value}"))?;
    let (time, offset_minutes) = split_time_and_offset(time, value)?;
    let mut d = date.split('-');
    let year: i32 = parse_part(d.next(), "year", value)?;
    let month: u32 = parse_part(d.next(), "month", value)?;
    let day: u32 = parse_part(d.next(), "day", value)?;

    let mut t = time.split(':');
    let hour: u32 = parse_part(t.next(), "hour", value)?;
    let minute: u32 = parse_part(t.next(), "minute", value)?;
    let sec_part = t
        .next()
        .ok_or_else(|| format!("timestamp is missing seconds: {value}"))?;
    let (second_raw, millis_raw) = sec_part.split_once('.').unwrap_or((sec_part, "0"));
    let second: u32 = second_raw
        .parse()
        .map_err(|_| format!("invalid seconds in timestamp: {value}"))?;
    let millis = parse_millis(millis_raw)?;

    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(format!("timestamp component out of range: {value}"));
    }

    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour as i64 * 3_600 + minute as i64 * 60 + second as i64;
    Ok(seconds * 1_000 + millis as i64 - offset_minutes * 60_000)
}

fn split_time_and_offset<'a>(time: &'a str, original: &str) -> Result<(&'a str, i64), String> {
    if time.len() < 2 {
        return Ok((time, 0));
    }
    let Some(offset_idx) = time[1..]
        .find(|ch| ch == '+' || ch == '-')
        .map(|idx| idx + 1)
    else {
        return Ok((time, 0));
    };
    let (clock, offset) = time.split_at(offset_idx);
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let offset = &offset[1..];
    let (hours, minutes) = offset
        .split_once(':')
        .ok_or_else(|| format!("invalid timezone offset in timestamp: {original}"))?;
    let hours: i64 = hours
        .parse()
        .map_err(|_| format!("invalid timezone offset hour in timestamp: {original}"))?;
    let minutes: i64 = minutes
        .parse()
        .map_err(|_| format!("invalid timezone offset minute in timestamp: {original}"))?;
    if hours > 23 || minutes > 59 {
        return Err(format!(
            "timezone offset out of range in timestamp: {original}"
        ));
    }
    Ok((clock, sign * (hours * 60 + minutes)))
}

fn parse_part<T>(part: Option<&str>, name: &str, timestamp: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    part.ok_or_else(|| format!("timestamp is missing {name}: {timestamp}"))?
        .parse()
        .map_err(|_| format!("invalid {name} in timestamp: {timestamp}"))
}

fn parse_millis(raw: &str) -> Result<u32, String> {
    let mut millis = 0_u32;
    let mut digits = 0;
    for ch in raw.chars().take(3) {
        let digit = ch
            .to_digit(10)
            .ok_or_else(|| format!("invalid millisecond digit: {raw}"))?;
        millis = millis * 10 + digit;
        digits += 1;
    }
    while digits < 3 {
        millis *= 10;
        digits += 1;
    }
    Ok(millis)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) as i64
}

pub fn seconds_between(start_ms: i64, end_ms: i64) -> f64 {
    ((end_ms - start_ms).max(0) as f64) / 1_000.0
}

pub fn whole_seconds_between(start_ms: i64, end_ms: i64) -> usize {
    ((end_ms - start_ms).max(0) / 1_000) as usize
}
