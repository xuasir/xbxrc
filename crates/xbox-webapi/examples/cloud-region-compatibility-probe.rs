use serde_json::{json, Value};
use xbox_webapi::{AuthApi, XcloudApi};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let Some(path) = std::env::args().nth(1) else {
        return;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let Ok(settings) = serde_json::from_slice::<Value>(&bytes) else {
        return;
    };
    let core = &settings["auth.tokens.core"];
    let Some(user_token) = core["sisuToken"]["UserToken"]["Token"].as_str() else {
        println!("{}", json!({"stage":"credentials","available":false}));
        return;
    };
    let private_jwk = &core["jwtKeys"]["privateJwk"];
    let route = settings["force_region_ip"]
        .as_str()
        .unwrap_or_default()
        .trim();
    let api = AuthApi::new();
    let token = match api
        .xsts_authorize(user_token, "http://gssv.xboxlive.com/", private_jwk)
        .await
    {
        Ok(token) => token.Token,
        Err(error) => {
            println!("{}", json!({"stage":"xsts","error":error.safe_summary()}));
            return;
        }
    };
    let result = api.get_streaming_token(&token, "xgpuweb", route).await;
    let value = match result {
        Ok(result) => result.data,
        Err(error) => {
            println!(
                "{}",
                json!({"stage":"production-login","error":error.safe_summary()})
            );
            return;
        }
    };
    let regions = value["offeringSettings"]["regions"].as_array().unwrap();
    println!(
        "{}",
        json!({"stage":"production-login","success":true,"regions":regions.len()})
    );
    let region = regions
        .iter()
        .find(|region| region["isDefault"] == true)
        .or_else(|| regions.first())
        .unwrap();
    let cloud = XcloudApi::new(
        region["baseUri"].as_str().unwrap().to_owned(),
        value["gsToken"].as_str().unwrap().to_owned(),
    );
    match cloud.get_titles().await {
        Ok(value) => println!(
            "{}",
            json!({"stage":"production-titles","success":true,"results":value["results"].as_array().map_or(0,Vec::len)})
        ),
        Err(error) => println!(
            "{}",
            json!({"stage":"production-titles","success":false,"error":error.safe_summary()})
        ),
    }
}
