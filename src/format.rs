use num_integer::Integer;

fn push_digit(res: &mut String, num: i64) {
    debug_assert!(num >= 0 && num < 10);
    res.push((b'0' + num as u8) as char);
}

pub fn format_timestamp(seconds: i64) -> String {
    let mut res = String::with_capacity(8);

    let (hours, seconds) = seconds.div_rem(&3600);
    if hours > 0 {
        let (tens, units) = hours.div_rem(&10);
        if tens > 0 {
            push_digit(&mut res, tens);
        }
        push_digit(&mut res, units);
        res += ":";
    }

    let (minutes, seconds) = seconds.div_rem(&60);
    let (tens, units) = minutes.div_rem(&10);
    if hours > 0 || tens != 0 {
        push_digit(&mut res, tens);
    }
    push_digit(&mut res, units);
    res += ":";

    let (tens, units) = seconds.div_rem(&10);
    push_digit(&mut res, tens);
    push_digit(&mut res, units);
    res
}
