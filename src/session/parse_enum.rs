use crate::session::UnknownEnumValueError;

pub trait ProtoEnum<Proto> {
    /// Returns a default value for given Protobuf definition.
    fn default() -> Proto;

    /// Returns value of the enum or None if the input `i32` does not describe
    /// any know enum value.
    /// `::prost::Enumeration` cannot be used instead, because it's not a trait.
    fn from_i32(val: i32) -> Option<Proto>;
}

/// Maps the raw integer value to enum value or returns `ParseError` when
/// the value cannot be mapped.
pub fn parse_enum<T: ProtoEnum<T>>(raw_enum_value: Option<i32>)
    -> Result<T, UnknownEnumValueError> {
    match raw_enum_value {
        Some(int_value) => match T::from_i32(int_value) {
            Some(parsed_value) => Ok(parsed_value),
            None => Err(UnknownEnumValueError {
                name: std::any::type_name::<T>(),
                value: int_value,
            }),
        },
        None => Ok(T::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, ::prost::Enumeration)]
    pub enum TestEnum {
        Default = 1,
        Two = 2,
    }

    impl ProtoEnum<TestEnum> for TestEnum {
        fn default() -> TestEnum { TestEnum::Default }
        fn from_i32(val: i32) -> Option<TestEnum> {
            TestEnum::from_i32(val)
        }
    }

    #[test]
    fn parse_empty_enum_test() {
        let empty: Option<i32> = None;
        let parsed : TestEnum = parse_enum(empty).unwrap();
        assert_eq!(parsed, TestEnum::Default);
    }

    #[test]
    fn parse_correct_enum_value_test() {
        let parsed : TestEnum = parse_enum(Some(2)).unwrap();
        assert_eq!(parsed, TestEnum::Two);
    }

    #[test]
    fn parse_incorrect_enum_value_test() {
        let parsed : Result<TestEnum, UnknownEnumValueError> = parse_enum(Some(3));
        let error = parsed.unwrap_err();
        assert_eq!(error.name, std::any::type_name::<TestEnum>());
        assert_eq!(error.value, 3);
    }
}
