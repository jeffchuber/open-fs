use regex::Regex;
use std::env;

use crate::ConfigError;

/// Interpolate environment variables in a string.
/// Replaces `${VAR_NAME}` with the value of the environment variable.
pub fn interpolate_env(input: &str) -> Result<String, ConfigError> {
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();

    let mut result = input.to_string();
    let mut errors = Vec::new();

    // Find all matches first to avoid mutation during iteration
    let matches: Vec<_> = re
        .captures_iter(input)
        .map(|cap| {
            let full_match = cap.get(0).unwrap().as_str().to_string();
            let var_name = cap.get(1).unwrap().as_str().to_string();
            (full_match, var_name)
        })
        .collect();

    for (full_match, var_name) in matches {
        match env::var(&var_name) {
            Ok(value) => {
                result = result.replace(&full_match, &value);
            }
            Err(_) => {
                errors.push(var_name);
            }
        }
    }

    if !errors.is_empty() {
        return Err(ConfigError::MissingEnvVars(errors));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_env() {
        env::set_var("TEST_VAR", "hello");
        env::set_var("ANOTHER_VAR", "world");

        let input = "prefix ${TEST_VAR} middle ${ANOTHER_VAR} suffix";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "prefix hello middle world suffix");
    }

    #[test]
    fn test_interpolate_env_missing() {
        let input = "prefix ${MISSING_VAR_12345} suffix";
        let result = interpolate_env(input);
        assert!(result.is_err());
        match result {
            Err(ConfigError::MissingEnvVars(vars)) => {
                assert_eq!(vars, vec!["MISSING_VAR_12345"]);
            }
            _ => panic!("Expected MissingEnvVars error"),
        }
    }

    #[test]
    fn test_interpolate_env_no_vars() {
        let input = "no variables here";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "no variables here");
    }

    #[test]
    fn test_interpolate_env_empty_string() {
        let input = "";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_interpolate_env_multiple_same_var() {
        env::set_var("REPEAT_VAR", "value");
        let input = "${REPEAT_VAR} and ${REPEAT_VAR} again";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "value and value again");
    }

    #[test]
    fn test_interpolate_env_adjacent_vars() {
        env::set_var("VAR_A", "hello");
        env::set_var("VAR_B", "world");
        let input = "${VAR_A}${VAR_B}";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "helloworld");
    }

    #[test]
    fn test_interpolate_env_var_at_start() {
        env::set_var("START_VAR", "start");
        let input = "${START_VAR} rest of string";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "start rest of string");
    }

    #[test]
    fn test_interpolate_env_var_at_end() {
        env::set_var("END_VAR", "end");
        let input = "beginning of string ${END_VAR}";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "beginning of string end");
    }

    #[test]
    fn test_interpolate_env_only_var() {
        env::set_var("ONLY_VAR", "only");
        let input = "${ONLY_VAR}";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "only");
    }

    #[test]
    fn test_interpolate_env_underscore_in_name() {
        env::set_var("VAR_WITH_UNDERSCORE", "value");
        let input = "${VAR_WITH_UNDERSCORE}";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "value");
    }

    #[test]
    fn test_interpolate_env_numbers_in_name() {
        env::set_var("VAR123", "numbers");
        let input = "${VAR123}";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "numbers");
    }

    #[test]
    fn test_interpolate_env_multiple_missing() {
        let input = "${MISSING_ONE_12345} and ${MISSING_TWO_12345}";
        let result = interpolate_env(input);
        assert!(result.is_err());
        match result {
            Err(ConfigError::MissingEnvVars(vars)) => {
                assert!(vars.contains(&"MISSING_ONE_12345".to_string()));
                assert!(vars.contains(&"MISSING_TWO_12345".to_string()));
            }
            _ => panic!("Expected MissingEnvVars error"),
        }
    }

    #[test]
    fn test_interpolate_env_partial_syntax_not_matched() {
        // Single $ without braces should not be matched
        let input = "not a $VAR variable";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "not a $VAR variable");
    }

    #[test]
    fn test_interpolate_env_unclosed_brace_not_matched() {
        // Unclosed brace should not be matched
        let input = "not a ${VAR variable";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "not a ${VAR variable");
    }

    #[test]
    fn test_interpolate_env_empty_value() {
        env::set_var("EMPTY_VAR", "");
        let input = "prefix ${EMPTY_VAR} suffix";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "prefix  suffix");
    }

    #[test]
    fn test_interpolate_env_special_chars_in_value() {
        env::set_var("SPECIAL_VAR", "value with $pecial ch@rs!");
        let input = "${SPECIAL_VAR}";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "value with $pecial ch@rs!");
    }

    #[test]
    fn test_interpolate_env_newlines_in_value() {
        env::set_var("MULTILINE_VAR", "line1\nline2\nline3");
        let input = "${MULTILINE_VAR}";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "line1\nline2\nline3");
    }

    #[test]
    fn test_interpolate_env_in_yaml_context() {
        env::set_var("YAML_ROOT", "/data/files");
        let input = "root: ${YAML_ROOT}/subdir";
        let result = interpolate_env(input).unwrap();
        assert_eq!(result, "root: /data/files/subdir");
    }
}
