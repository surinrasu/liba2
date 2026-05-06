pub(crate) fn generate_device_id() -> String {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).expect("operating system random source failed");

    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}
