#[cfg(test)]
pub mod test_utils {
    use log::LevelFilter;
    use std::sync::Once;

    // 使用 Once 确保 env_logger 只初始化一次
    static INIT: Once = Once::new();

    pub fn setup() {
        INIT.call_once(|| {
            env_logger::builder().filter_level(LevelFilter::Info).init();
        });
    }
}
