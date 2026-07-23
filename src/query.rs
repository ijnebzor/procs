use anyhow::{Context, Error, anyhow};
use jaq_all::data::{self, Runner};
use jaq_all::jaq_core::{Vars, unwrap_valr};
use jaq_all::json::Val;
use jaq_all::{compile_with, defs, load};

pub struct WhereFilter {
    filter: data::Filter,
    runner: Runner,
}

impl WhereFilter {
    pub fn compile(expression: &str) -> Result<Self, Error> {
        let filter = compile_with(expression, defs(), data::funs(), &[]).map_err(|reports| {
            let details = reports
                .iter()
                .map(|report| load::FileReportsDisp::new(report).to_string())
                .collect::<Vec<_>>()
                .join("\n");
            anyhow!("invalid --where expression:\n{details}")
        })?;

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

#[cfg(test)]
mod tests {
    use super::WhereFilter;
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
}
