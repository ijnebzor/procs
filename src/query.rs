use anyhow::{Context, Error, anyhow};
use jaq_all::data::{self, Runner};
use jaq_all::jaq_core::{Vars, unwrap_valr};
use jaq_all::json::Val;
use jaq_all::{compile_with, defs, load};

struct CompiledQuery {
    filter: data::Filter,
    runner: Runner,
}

impl CompiledQuery {
    fn compile(expression: &str, option: &str) -> Result<Self, Error> {
        let filter = compile_with(expression, defs(), data::funs(), &[]).map_err(|reports| {
            let details = reports
                .iter()
                .map(|report| load::FileReportsDisp::new(report).to_string())
                .collect::<Vec<_>>()
                .join("\n");
            anyhow!("invalid {option} expression:\n{details}")
        })?;

        Ok(Self {
            filter,
            runner: Runner::default(),
        })
    }

    fn run(&self, value: &serde_json::Value, option: &str) -> Result<Vec<Val>, Error> {
        let input: Val = serde_json::from_value(value.clone())
            .with_context(|| format!("failed to convert canonical JSON input for {option}"))?;
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
                outputs.push(output);
                Ok(())
            },
        )
        .with_context(|| format!("failed to evaluate {option} expression"))?;

        Ok(outputs)
    }
}

pub struct WhereFilter {
    query: CompiledQuery,
}

impl WhereFilter {
    pub fn compile(expression: &str) -> Result<Self, Error> {
        Ok(Self {
            query: CompiledQuery::compile(expression, "--where")?,
        })
    }

    pub fn matches(&self, value: &serde_json::Value) -> Result<bool, Error> {
        let outputs = self.query.run(value, "--where")?;
        Ok(outputs
            .into_iter()
            .any(|output| !matches!(output, Val::Null | Val::Bool(false))))
    }
}

pub struct JqTransform {
    query: CompiledQuery,
}

impl JqTransform {
    pub fn compile(expression: &str) -> Result<Self, Error> {
        Ok(Self {
            query: CompiledQuery::compile(expression, "--jq")?,
        })
    }

    pub fn transform(&self, value: &serde_json::Value) -> Result<Vec<serde_json::Value>, Error> {
        self.query
            .run(value, "--jq")?
            .into_iter()
            .map(|output| {
                let output = output.to_string();
                serde_json::from_str(&output).map_err(|error| {
                    anyhow!("--jq produced a value that cannot be encoded as JSON: {error}")
                })
            })
            .collect()
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
    fn transforms_complete_arrays_and_preserves_multiple_outputs() {
        let transform =
            JqTransform::compile("sort_by(.usage_cpu) | reverse | .[:2] | .[]").unwrap();
        let outputs = transform
            .transform(&json!([
                {"pid": 1, "usage_cpu": 2.0},
                {"pid": 2, "usage_cpu": 10.0},
                {"pid": 3, "usage_cpu": 5.0}
            ]))
            .unwrap();

        assert_eq!(
            outputs,
            [
                json!({"pid": 2, "usage_cpu": 10.0}),
                json!({"pid": 3, "usage_cpu": 5.0})
            ]
        );
    }

    #[test]
    fn transform_supports_empty_and_null_outputs() {
        let empty = JqTransform::compile("empty").unwrap();
        let null = JqTransform::compile("null").unwrap();
        let input = json!([{"pid": 1}]);

        assert!(empty.transform(&input).unwrap().is_empty());
        assert_eq!(null.transform(&input).unwrap(), [serde_json::Value::Null]);
    }

    #[test]
    fn transform_reports_invalid_expressions() {
        let error = JqTransform::compile("map(").err().unwrap();
        assert!(error.to_string().contains("invalid --jq expression"));
    }

    #[test]
    fn transform_preserves_false_and_null_outputs() {
        let transform = JqTransform::compile("false, null").unwrap();
        assert_eq!(
            transform.transform(&json!([])).unwrap(),
            [json!(false), serde_json::Value::Null]
        );
    }

    #[test]
    fn transform_handles_empty_input_arrays() {
        let length = JqTransform::compile("length").unwrap();
        let elements = JqTransform::compile(".[]").unwrap();
        let input = json!([]);

        assert_eq!(length.transform(&input).unwrap(), [json!(0)]);
        assert!(elements.transform(&input).unwrap().is_empty());
    }

    #[test]
    fn transform_uses_jq_null_semantics_for_missing_fields() {
        let transform = JqTransform::compile("map(.missing)").unwrap();
        assert_eq!(
            transform
                .transform(&json!([{"pid": 1}, {"pid": 2}]))
                .unwrap(),
            [json!([null, null])]
        );
    }

    #[test]
    fn transform_reports_runtime_errors() {
        let transform = JqTransform::compile("error(\"boom\")").unwrap();
        let error = transform.transform(&json!([])).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("failed to evaluate --jq expression")
        );
        assert!(format!("{error:#}").contains("boom"));
    }

    #[test]
    fn transform_rejects_jaq_values_that_are_not_valid_json() {
        let transform = JqTransform::compile("nan").unwrap();
        let error = transform.transform(&json!([])).err().unwrap();
        assert!(
            error
                .to_string()
                .contains("--jq produced a value that cannot be encoded as JSON")
        );
    }
}
