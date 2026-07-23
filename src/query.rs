use anyhow::{Context, Error, anyhow};
use jaq_all::data::{self, Runner};
use jaq_all::jaq_core::{Vars, unwrap_valr};
use jaq_all::json::Val;
use jaq_all::{compile_with, defs, load};

fn compile(expression: &str, option: &str) -> Result<data::Filter, Error> {
    compile_with(expression, defs(), data::funs(), &[]).map_err(|reports| {
        let details = reports
            .iter()
            .map(|report| load::FileReportsDisp::new(report).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        anyhow!("invalid {option} expression:\n{details}")
    })
}

pub struct WhereFilter {
    filter: data::Filter,
    runner: Runner,
}

impl WhereFilter {
    pub fn compile(expression: &str) -> Result<Self, Error> {
        let filter = compile(expression, "--where")?;

        Ok(Self {
            filter,
            runner: Runner::default(),
        })
    }

    pub fn matches(&self, value: &serde_json::Value) -> Result<bool, Error> {
        let input: Val = serde_json::from_value(value.clone())
            .context("failed to convert canonical process record for --where")?;
        let inputs = std::iter::once(Ok::<Val, String>(input));
        let mut matched = false;

        data::run(
            &self.runner,
            &self.filter,
            Vars::new([]),
            inputs,
            |message| anyhow!(message),
            |output| {
                let output = unwrap_valr(output).map_err(|error| anyhow!(error.to_string()))?;
                if !matches!(output, Val::Null | Val::Bool(false)) {
                    matched = true;
                }
                Ok(())
            },
        )?;

        Ok(matched)
    }
}

pub struct JqTransform {
    filter: data::Filter,
    runner: Runner,
}

impl JqTransform {
    pub fn compile(expression: &str) -> Result<Self, Error> {
        let filter = compile(expression, "--jq")?;

        Ok(Self {
            filter,
            runner: Runner::default(),
        })
    }

    /// Apply the filter once to the complete canonical result.
    ///
    /// The returned vector preserves jq's output stream: it may contain zero,
    /// one, or multiple JSON values in evaluation order.
    pub fn transform(&self, value: &serde_json::Value, pretty: bool) -> Result<Vec<String>, Error> {
        let input: Val = serde_json::from_value(value.clone())
            .context("failed to convert canonical result for --jq")?;
        let inputs = std::iter::once(Ok::<Val, String>(input));
        let mut outputs = Vec::new();

        data::run(
            &self.runner,
            &self.filter,
            Vars::new([]),
            inputs,
            |message| anyhow!(message),
            |output| {
                let output = unwrap_valr(output).map_err(|error| anyhow!(error.to_string()))?;
                let mut printer = jaq_all::fmts::write::json::Pp::default();
                if pretty {
                    printer.indent = Some("  ".into());
                    printer.sep_space = true;
                }
                let mut json = Vec::new();
                jaq_all::fmts::write::json::write(&mut json, &printer, 0, &output)?;
                serde_json::from_slice::<serde_json::Value>(&json)
                    .context("--jq produced a value that cannot be represented as JSON")?;
                outputs.push(
                    String::from_utf8(json).context("--jq produced invalid UTF-8 JSON output")?,
                );
                Ok(())
            },
        )
        .context("failed to evaluate --jq expression")?;

        Ok(outputs)
    }
}

#[cfg(test)]
mod tests {
    use super::{JqTransform, WhereFilter};
    use serde_json::json;

    #[test]
    fn compiles_and_matches_truthy_results() {
        let filter = WhereFilter::compile(".usage_cpu > 10 and .user == \"alice\"").unwrap();

        assert!(
            filter
                .matches(&json!({"usage_cpu": 12.5, "user": "alice"}))
                .unwrap()
        );
        assert!(
            !filter
                .matches(&json!({"usage_cpu": 3.0, "user": "alice"}))
                .unwrap()
        );
    }

    #[test]
    fn no_output_and_false_are_not_matches() {
        let empty = WhereFilter::compile("empty").unwrap();
        let false_value = WhereFilter::compile("false").unwrap();
        let input = json!({"pid": 1});

        assert!(!empty.matches(&input).unwrap());
        assert!(!false_value.matches(&input).unwrap());
    }

    #[test]
    fn reports_invalid_expressions() {
        let error = WhereFilter::compile(".usage_cpu >").err().unwrap();
        assert!(error.to_string().contains("invalid --where expression"));
    }

    #[test]
    fn jq_transforms_the_complete_array_to_an_object() {
        let transform = JqTransform::compile("{pids: map(.pid), count: length}").unwrap();

        let outputs = transform
            .transform(&json!([{"pid": 10}, {"pid": 20}]), false)
            .unwrap();

        assert_eq!(outputs, [r#"{"pids":[10,20],"count":2}"#]);
    }

    #[test]
    fn jq_preserves_array_object_and_scalar_output_streams() {
        let transform =
            JqTransform::compile("map(.pid), {count: length}, length, true, null").unwrap();

        let outputs = transform
            .transform(&json!([{"pid": 10}, {"pid": 20}]), false)
            .unwrap();

        assert_eq!(outputs, ["[10,20]", r#"{"count":2}"#, "2", "true", "null"]);
    }

    #[test]
    fn jq_preserves_empty_output_streams() {
        let transform = JqTransform::compile("empty").unwrap();

        assert!(
            transform
                .transform(&json!([{"pid": 10}]), false)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn jq_reports_invalid_expressions() {
        let error = JqTransform::compile("map(").err().unwrap();

        assert!(error.to_string().contains("invalid --jq expression"));
    }

    #[test]
    fn jq_reports_runtime_errors() {
        let transform = JqTransform::compile(".[0].pid | .name").unwrap();
        let error = transform
            .transform(&json!([{"pid": 10}]), false)
            .unwrap_err();
        let diagnostic = error
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(diagnostic.contains("failed to evaluate --jq expression"));
        assert!(diagnostic.contains("cannot index"));
    }

    #[test]
    fn jq_rejects_query_engine_values_that_are_not_valid_json() {
        let transform = JqTransform::compile("nan").unwrap();
        let error = transform.transform(&json!([]), false).unwrap_err();
        let diagnostic = error
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(diagnostic.contains("failed to evaluate --jq expression"));
        assert!(diagnostic.contains("cannot be represented as JSON"));
    }

    #[test]
    fn jq_serialization_preserves_large_numbers_and_supports_pretty_output() {
        let transform =
            JqTransform::compile("999999999999999999999999999999999999999999, {answer: 42}")
                .unwrap();
        let outputs = transform.transform(&json!([]), true).unwrap();

        assert_eq!(outputs[0], "999999999999999999999999999999999999999999");
        assert_eq!(outputs[1], "{\n  \"answer\": 42\n}");
    }
}
