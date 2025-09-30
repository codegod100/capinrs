use serde_json::{json, Value};
use std::collections::VecDeque;
use wasm_bindgen::prelude::*;

const CALCULATOR_CAP_ID: u64 = 1;

#[derive(Debug)]
enum PendingOutcome {
    Result(Value),
    Error(String),
}

#[wasm_bindgen]
pub fn process_rpc(input: &str) -> Result<String, JsValue> {
    process_batch(input).map_err(|err| JsValue::from_str(&err))
}

fn process_batch(input: &str) -> Result<String, String> {
    let mut pending = VecDeque::<PendingOutcome>::new();
    let mut responses: Vec<String> = Vec::new();

    for (line_number, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let op: Value = serde_json::from_str(line)
            .map_err(|err| format!("line {}: failed to parse JSON: {}", line_number + 1, err))?;
        let arr = op
            .as_array()
            .ok_or_else(|| format!("line {}: expected array operation", line_number + 1))?;

        let kind = arr
            .get(0)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("line {}: operation tag must be a string", line_number + 1))?;

        match kind {
            "push" => {
                let payload = arr.get(1).ok_or_else(|| {
                    format!("line {}: push operation missing payload", line_number + 1)
                })?;
                handle_push(payload, &mut pending).map_err(|err| {
                    format!("line {}: {}", line_number + 1, err)
                })?;
            }
            "pull" => {
                let import_id = arr
                    .get(1)
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| format!("line {}: pull expects numeric import id", line_number + 1))?;

                let outcome = pending.pop_front().unwrap_or_else(|| {
                    PendingOutcome::Error("no pending result for pull".to_string())
                });

                let message = match outcome {
                    PendingOutcome::Result(value) => json!(["result", import_id, value]),
                    PendingOutcome::Error(message) => json!([
                        "error",
                        import_id,
                        {
                            "message": message,
                        }
                    ]),
                };

                responses.push(
                    serde_json::to_string(&message)
                        .map_err(|err| format!("failed to serialize response: {}", err))?,
                );
            }
            other => {
                return Err(format!("line {}: unsupported operation `{}`", line_number + 1, other));
            }
        }
    }

    Ok(responses.join("\n"))
}

fn handle_push(payload: &Value, pending: &mut VecDeque<PendingOutcome>) -> Result<(), String> {
    let arr = payload
        .as_array()
        .ok_or_else(|| "push payload must be an array".to_string())?;

    let op_kind = arr
        .get(0)
        .and_then(|v| v.as_str())
        .ok_or_else(|| "push payload kind must be a string".to_string())?;

    match op_kind {
        "call" => {
            let cap_id = arr
                .get(1)
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "call operation missing numeric capability id".to_string())?;

            if cap_id != CALCULATOR_CAP_ID {
                pending.push_back(PendingOutcome::Error(format!(
                    "capability `{}` is not registered",
                    cap_id
                )));
                return Ok(());
            }

            let path = arr
                .get(2)
                .and_then(|v| v.as_array())
                .ok_or_else(|| "call operation must include a method path array".to_string())?;

            let method = path
                .first()
                .and_then(|v| v.as_str())
                .ok_or_else(|| "call method name must be a string".to_string())?;

            let args: Vec<Value> = match arr.get(3) {
                Some(Value::Array(values)) => values.clone(),
                Some(_) => return Err("call arguments must be an array".to_string()),
                None => Vec::new(),
            };

            match invoke_calculator(method, &args) {
                Ok(value) => pending.push_back(PendingOutcome::Result(value)),
                Err(err) => pending.push_back(PendingOutcome::Error(err)),
            }
        }
        other => {
            pending.push_back(PendingOutcome::Error(format!(
                "unsupported push operation `{}`",
                other
            )));
        }
    }

    Ok(())
}

fn invoke_calculator(method: &str, args: &[Value]) -> Result<Value, String> {
    match method {
        "add" => {
            if args.len() != 2 {
                return Err("`add` expects exactly two numeric arguments".into());
            }

            let a = args[0]
                .as_f64()
                .ok_or_else(|| "first argument must be a number".to_string())?;
            let b = args[1]
                .as_f64()
                .ok_or_else(|| "second argument must be a number".to_string())?;

            Ok(json!(a + b))
        }
        other => Err(format!("unknown method `{}`", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn run_batch(input: &str) -> Result<Vec<Value>, String> {
        process_batch(input).map(|output| {
            output
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        })
    }

    #[test]
    fn happy_path_add() {
        let batch = r#"
            ["push", ["call", 1, ["add"], [10, 20]]]
            ["pull", 1]
        "#;

        let responses = run_batch(batch).unwrap();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0], json!(["result", 1, 30.0]));
    }

    #[test]
    fn invalid_method() {
        let batch = r#"
            ["push", ["call", 1, ["subtract"], [10, 20]]]
            ["pull", 5]
        "#;

        let responses = run_batch(batch).unwrap();
        assert_eq!(responses[0][0], json!("error"));
        assert_eq!(responses[0][1], json!(5));
    }

    #[test]
    fn malformed_json() {
        let batch = "not json";
        let err = process_batch(batch).unwrap_err();
        assert!(err.contains("failed to parse JSON"));
    }
}
