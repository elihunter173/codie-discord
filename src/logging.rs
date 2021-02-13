use core::fmt;

pub trait Loggable<'a> {
    type Log: fmt::Display;
    fn as_log(&'a self) -> Self::Log;
}

// Wish I could impl fmt::Display directly on shiplift::Container
pub struct LogContainer<'a>(&'a shiplift::Container<'a>);
impl fmt::Display for LogContainer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Container(id={})", self.0.id())
    }
}

impl<'a> Loggable<'a> for shiplift::Container<'a> {
    type Log = LogContainer<'a>;
    fn as_log(&'a self) -> Self::Log {
        LogContainer(self)
    }
}
