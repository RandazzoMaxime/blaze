use std::fmt::{self, Display};

/// Convert special characters into XML entities.
/// This is needed for checkstyle output.
pub(super) struct XmlEscaped<'a>(pub(super) &'a str);

impl<'a> Display for XmlEscaped<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for char in self.0.chars() {
            match char {
                '<' => write!(formatter, "&lt;"),
                '>' => write!(formatter, "&gt;"),
                '"' => write!(formatter, "&quot;"),
                '\'' => write!(formatter, "&apos;"),
                '&' => write!(formatter, "&amp;"),
                _ => write!(formatter, "{char}"),
            }?;
        }

        Ok(())
    }
}
