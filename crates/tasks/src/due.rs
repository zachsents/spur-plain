//! When items are due, read the way people say it: "today", "tomorrow", "Friday", "next week",
//! "in 3 days", "Nov 20", "11/20" or "2026-11-20". The parser is local and instant, so people and
//! agents are read the same, and what it can't read goes to the suite's decision model.

use gpui::{App, Task};
use jiff::{
    Zoned,
    civil::{Date, Weekday},
};
use schemars::JsonSchema;
use serde::Deserialize;

const HINT: &str = "Try today, Friday, in 3 days or Nov 20";

/// What someone typed for a due date: a date, or no date at all ("none").
pub type Due = Option<Date>;

/// Reads `input` as a due date, counting from `today`.
pub fn parse(input: &str, today: Date) -> Result<Due, String> {
    let text = input
        .trim()
        .to_lowercase()
        .replace([',', '.'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let text = text.strip_prefix("due ").unwrap_or(&text);
    let text = text.strip_prefix("on ").unwrap_or(text);
    if matches!(
        text,
        "" | "none" | "no date" | "no due date" | "never" | "clear" | "remove" | "nothing"
    ) {
        return Ok(None);
    }
    let day = |days: i64| today.checked_add(jiff::Span::new().days(days)).ok();
    let named = match text {
        "today" | "tonight" | "now" | "tod" => day(0),
        "tomorrow" | "tmrw" | "tmr" | "tom" | "tomorrow morning" | "tomorrow night" => day(1),
        "yesterday" => day(-1),
        "next week" => monday_after(today),
        "weekend" | "this weekend" => coming(today, Weekday::Saturday, false),
        "next weekend" => monday_after(today)
            .and_then(|monday| monday.checked_add(jiff::Span::new().days(5)).ok()),
        "next month" => today.checked_add(jiff::Span::new().months(1)).ok(),
        "end of month" | "end of the month" => Some(today.last_of_month()),
        _ => None,
    };
    if let Some(date) = named {
        return Ok(Some(date));
    }
    weekday(text, today)
        .or_else(|| offset(text, today))
        .or_else(|| written_date(text, today))
        .map(Some)
        .ok_or_else(|| HINT.to_string())
}

/// "Friday" or "this fri" is the coming one, today included. "Next Friday" is the one in the
/// week after this one.
fn weekday(text: &str, today: Date) -> Option<Date> {
    let (next, name) = match text.strip_prefix("next ") {
        Some(name) => (true, name),
        None => (false, text.strip_prefix("this ").unwrap_or(text)),
    };
    let weekday = weekday_named(name)?;
    if next {
        let monday = monday_after(today)?;
        let offset = i64::from(weekday.to_monday_zero_offset());
        return monday.checked_add(jiff::Span::new().days(offset)).ok();
    }
    coming(today, weekday, true)
}

fn weekday_named(name: &str) -> Option<Weekday> {
    Some(match name {
        "mon" | "monday" => Weekday::Monday,
        "tue" | "tues" | "tuesday" => Weekday::Tuesday,
        "wed" | "weds" | "wednesday" => Weekday::Wednesday,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thursday,
        "fri" | "friday" => Weekday::Friday,
        "sat" | "saturday" => Weekday::Saturday,
        "sun" | "sunday" => Weekday::Sunday,
        _ => return None,
    })
}

/// The next `weekday` from `today`, which counts if `including_today`.
fn coming(today: Date, weekday: Weekday, including_today: bool) -> Option<Date> {
    let ahead = (i64::from(weekday.to_monday_zero_offset())
        - i64::from(today.weekday().to_monday_zero_offset()))
    .rem_euclid(7);
    let ahead = if ahead == 0 && !including_today {
        7
    } else {
        ahead
    };
    today.checked_add(jiff::Span::new().days(ahead)).ok()
}

/// The Monday that starts next week.
fn monday_after(today: Date) -> Option<Date> {
    coming(today, Weekday::Monday, false)
}

/// "in 3 days", "a week", "in two weeks", "2 months".
fn offset(text: &str, today: Date) -> Option<Date> {
    let text = text.strip_prefix("in ").unwrap_or(text);
    let text = text.strip_suffix(" from now").unwrap_or(text);
    let (count, unit) = text.split_once(' ')?;
    let count = match count {
        "a" | "an" | "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        number => number.parse::<i64>().ok()?,
    };
    let span = match unit.trim_end_matches('s') {
        "day" => jiff::Span::new().days(count),
        "week" | "wk" => jiff::Span::new().weeks(count),
        "month" | "mo" => jiff::Span::new().months(count),
        "year" | "yr" => jiff::Span::new().years(count),
        _ => return None,
    };
    today.checked_add(span).ok()
}

/// "Nov 20", "20 november", "november 20th 2026", "11/20", "11/20/26", "2026-11-20", "the 20th".
/// A month and day without a year that has passed is next year's; a day of the month that has
/// passed is next month's.
fn written_date(text: &str, today: Date) -> Option<Date> {
    if let Ok(date) = text.parse::<Date>() {
        return Some(date);
    }
    if let Some(date) = slashed(text, today) {
        return Some(date);
    }
    let words: Vec<_> = text
        .split_whitespace()
        .filter(|word| *word != "the" && *word != "of")
        .collect();
    let number = |word: &str| -> Option<i64> {
        word.trim_end_matches(|letter: char| letter.is_ascii_alphabetic())
            .parse()
            .ok()
    };
    let month = words.iter().find_map(|word| month_named(word));
    let numbers: Vec<i64> = words.iter().filter_map(|word| number(word)).collect();
    match (month, numbers.as_slice()) {
        (Some(month), [day]) => upcoming(today, month, *day),
        (Some(month), [day, year]) => {
            Date::new(year_of(*year)?, month, i8::try_from(*day).ok()?).ok()
        }
        (None, [day]) if words.len() == 1 => {
            let this_month = Date::new(today.year(), today.month(), i8::try_from(*day).ok()?).ok();
            match this_month {
                Some(date) if date >= today => Some(date),
                _ => {
                    let next = today
                        .first_of_month()
                        .checked_add(jiff::Span::new().months(1))
                        .ok()?;
                    Date::new(next.year(), next.month(), i8::try_from(*day).ok()?).ok()
                }
            }
        }
        _ => None,
    }
}

fn slashed(text: &str, today: Date) -> Option<Date> {
    let parts: Vec<i64> = text
        .split('/')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [month, day] => upcoming(today, i8::try_from(*month).ok()?, *day),
        [month, day, year] => Date::new(
            year_of(*year)?,
            i8::try_from(*month).ok()?,
            i8::try_from(*day).ok()?,
        )
        .ok(),
        _ => None,
    }
}

/// A two-digit year is this century's.
fn year_of(year: i64) -> Option<i16> {
    i16::try_from(if year < 100 { 2000 + year } else { year }).ok()
}

/// The next time a month and day comes round, today included.
fn upcoming(today: Date, month: i8, day: i64) -> Option<Date> {
    let day = i8::try_from(day).ok()?;
    let this_year = Date::new(today.year(), month, day).ok()?;
    if this_year >= today {
        return Some(this_year);
    }
    Date::new(today.year() + 1, month, day).ok()
}

/// A month written out or shortened to three letters or more: "nov", "sept", "november".
fn month_named(word: &str) -> Option<i8> {
    const NAMES: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    if word.len() < 3 {
        return None;
    }
    let position = NAMES.iter().position(|name| name.starts_with(word))?;
    i8::try_from(position + 1).ok()
}

/// How a due date reads in a list, from `today`: "Today", "Tomorrow", "Yesterday", the
/// weekday within the coming week, else the date, with its year if it isn't this year's.
pub fn describe(due: Date, today: Date) -> String {
    let days = (due - today).get_days();
    match days {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        2..=6 => due.strftime("%A").to_string(),
        _ if due.year() == today.year() => due.strftime("%b %-d").to_string(),
        _ => due.strftime("%b %-d, %Y").to_string(),
    }
}

/// What the decision model reads a due date as.
#[derive(Deserialize, JsonSchema)]
struct Said {
    /// The date it's due, as YYYY-MM-DD, or null for no due date at all.
    date: Option<String>,
}

/// Reads `input` like [`parse`], asking the suite's decision model about anything the parser
/// can't read, like "the first Monday in December" or "before Thanksgiving". Without an AI key
/// only the parser's forms work.
pub fn understand(input: String, cx: &App) -> Task<Result<Due, String>> {
    let today = Zoned::now().date();
    if let Ok(due) = parse(&input, today) {
        return Task::ready(Ok(due));
    }
    cx.background_executor().spawn(async move {
        let key = ai::keys::find(ai::Provider::Gateway).ok_or(HINT)?;
        let task = format!(
            "Someone typed when a to-do item is due. Today is {}. Read it as a date.",
            today.strftime("%A, %B %-d, %Y")
        );
        let said: Said = ai::decide(&key, &task, &input)?;
        said.date
            .map(|date| date.parse::<Date>().map_err(|_| HINT.to_string()))
            .transpose()
    })
}
