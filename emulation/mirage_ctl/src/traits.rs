use mirage_schema::daemon::MirageDaemon;

pub trait CommandFunction {
    type Output: serde::Serialize;
    fn call(self, daemon: &impl MirageDaemon) -> Self::Output;
}
