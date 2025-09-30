use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[derive(Debug, Deserialize)]
struct RpcRequest {
    method: String,
    #[serde(default)]
    args: Vec<f64>,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    result: Option<f64>,
    error: Option<String>,
}

fn compute(request: RpcRequest) -> RpcResponse {
    match request.method.as_str() {
        "add" => {
            if request.args.len() != 2 {
                RpcResponse {
                    result: None,
                    error: Some("`add` expects exactly two numeric arguments".into()),
                }
            } else {
                let sum = request.args[0] + request.args[1];
                RpcResponse {
                    result: Some(sum),
                    error: None,
                }
            }
        }
        other => RpcResponse {
            result: None,
            error: Some(format!("unknown method `{}`", other)),
        },
    }
}

#[wasm_bindgen]
pub fn process_rpc(input: &str) -> String {
    let response = match serde_json::from_str::<RpcRequest>(input) {
        Ok(request) => compute(request),
        Err(err) => RpcResponse {
            result: None,
            error: Some(format!("invalid request: {}", err)),
        },
    };

    serde_json::to_string(&response).unwrap_or_else(|err| {
        serde_json::json!({
            "result": null,
            "error": format!("serialization error: {}", err),
        })
        .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_happy_path() {
        let req = RpcRequest {
            method: "add".into(),
            args: vec![1.0, 2.5],
        };
        let resp = compute(req);
        assert_eq!(resp.result, Some(3.5));
        assert_eq!(resp.error, None);
    }

    #[test]
    fn add_invalid_args() {
        let req = RpcRequest {
            method: "add".into(),
            args: vec![1.0],
        };
        let resp = compute(req);
        assert!(resp.result.is_none());
        assert!(
            resp.error
                .unwrap()
                .contains("exactly two numeric arguments")
        );
    }

    #[test]
    fn unknown_method() {
        let req = RpcRequest {
            method: "mul".into(),
            args: vec![1.0, 2.0],
        };
        let resp = compute(req);
        assert!(resp.result.is_none());
        assert!(resp.error.unwrap().contains("unknown method"));
    }
}
