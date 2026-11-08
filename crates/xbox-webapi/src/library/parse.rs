use super::types::*;
use crate::WebApiError;
use serde_json::Value;
use std::collections::HashSet;

pub fn library_root(v: &Value) -> &Value {
    v.get("data").unwrap_or(v)
}
pub fn library_string(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}
pub fn library_u64(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64().or_else(|| whole_u64(n.as_f64()?)),
        Value::String(s) => s
            .trim()
            .parse()
            .ok()
            .or_else(|| whole_u64(s.trim().parse().ok()?)),
        _ => None,
    }
}

fn whole_u64(value: f64) -> Option<u64> {
    (value.is_finite() && value >= 0.0 && value.fract() == 0.0 && value < u64::MAX as f64)
        .then_some(value as u64)
}

pub fn library_image_url(v: Option<&Value>) -> Option<String> {
    let value = library_string(v)?;
    let value = if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value
    };
    let mut url = reqwest::Url::parse(&value).ok()?;
    let host = url.host_str()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    // Xbox 的历史封面仍返回 HTTP，已验证第一方图片服务支持相同 HTTPS 地址。
    if url.scheme() == "http"
        && (host.ends_with(".xboxlive.com") || host.ends_with(".s-microsoft.com"))
    {
        url.set_scheme("https").ok()?;
    }
    (url.scheme() == "https").then(|| url.to_string())
}

fn count(v: Option<&Value>) -> Option<u32> {
    library_u64(v).and_then(|n| n.try_into().ok())
}
fn rows<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, WebApiError> {
    library_root(v)
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| WebApiError::parse(format!("missing {key} array")))
}
pub fn library_continuation(v: &Value) -> Option<String> {
    let v = library_root(v);
    library_string(v.get("pagingInfo").and_then(|p| p.get("continuationToken")))
        .or_else(|| library_string(v.get("continuationToken")))
        .or_else(|| library_string(v.get("continuation")))
}
pub fn parse_game_history(v: &Value) -> Result<LibraryPage<LibraryGame>, WebApiError> {
    let mut seen = HashSet::new();
    let items = rows(v, "titles")?
        .iter()
        .filter_map(|g| {
            if g.get("type")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.eq_ignore_ascii_case("Game"))
            {
                return None;
            }
            let id = library_string(g.get("titleId"))?;
            if !seen.insert(id.clone()) {
                return None;
            }
            let artwork_url = g
                .get("images")
                .and_then(Value::as_array)
                .and_then(|images| {
                    ["Tile", "BoxArt", "Poster", "FeaturePromotionalSquareArt"]
                        .iter()
                        .find_map(|kind| {
                            images
                                .iter()
                                .find(|i| {
                                    i.get("type")
                                        .and_then(Value::as_str)
                                        .is_some_and(|s| s.eq_ignore_ascii_case(kind))
                                })
                                .and_then(|i| library_image_url(i.get("url")))
                        })
                })
                .or_else(|| library_image_url(g.get("displayImage")));
            Some(LibraryGame {
                description: library_string(g.get("detail").and_then(|d| d.get("description"))),
                xbox_title_id: id,
                scid: library_string(g.get("serviceConfigId").or_else(|| g.get("scid"))),
                product_id: library_string(g.get("productId")),
                name: library_string(g.get("name"))?,
                artwork_url,
                last_played_at: library_string(
                    g.get("titleHistory").and_then(|h| h.get("lastTimePlayed")),
                ),
                progress: g
                    .get("achievement")
                    .filter(|a| a.is_object())
                    .filter(|a| {
                        // sourceVersion=0 的全零装饰表示服务未提供汇总。
                        count(a.get("sourceVersion")) != Some(0)
                            || [
                                "currentAchievements",
                                "totalAchievements",
                                "currentGamerscore",
                                "totalGamerscore",
                            ]
                            .iter()
                            .any(|key| count(a.get(*key)) != Some(0))
                    })
                    .map(|a| GameProgress {
                        unlocked_count: count(a.get("currentAchievements")),
                        total_count: count(a.get("totalAchievements")),
                        earned_gamerscore: count(a.get("currentGamerscore")),
                        total_gamerscore: count(a.get("totalGamerscore")),
                    }),
            })
        })
        .collect();
    Ok(LibraryPage {
        items,
        continuation: library_continuation(v),
    })
}
pub fn parse_achievement_page(v: &Value) -> Result<LibraryPage<LibraryAchievement>, WebApiError> {
    let mut seen = HashSet::new();
    let items = rows(v, "achievements")?
        .iter()
        .filter_map(|a| {
            let id = library_string(a.get("id"))?;
            let scid = library_string(a.get("serviceConfigId"));
            if !seen.insert((scid.clone(), id.clone())) {
                return None;
            }
            let is_unlocked = a
                .get("progressState")
                .and_then(Value::as_str)
                .is_some_and(|s| s.eq_ignore_ascii_case("Achieved"));
            let p = a.get("progression");
            let requirements: Vec<_> = p
                .and_then(|p| p.get("requirements"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|r| AchievementRequirement {
                    id: library_string(r.get("id")).unwrap_or_default(),
                    current: library_string(r.get("current")),
                    target: library_string(r.get("target")),
                })
                .collect();
            // 复合要求保留逐项数据；仅单一数值要求产生百分比。
            let progress_percentage = if is_unlocked {
                Some(100.0)
            } else if requirements.len() == 1 {
                requirements[0]
                    .current
                    .as_deref()
                    .and_then(|s| s.parse::<f64>().ok())
                    .zip(
                        requirements[0]
                            .target
                            .as_deref()
                            .and_then(|s| s.parse::<f64>().ok()),
                    )
                    .filter(|(c, t)| c.is_finite() && t.is_finite() && *t > 0.0)
                    .map(|(c, t)| (c / t * 100.0).clamp(0.0, 100.0))
            } else {
                None
            };
            Some(LibraryAchievement {
                id,
                scid,
                name: library_string(a.get("name")).unwrap_or_else(|| "Xbox Achievement".into()),
                description: library_string(a.get("description")).unwrap_or_default(),
                locked_description: library_string(a.get("lockedDescription")).unwrap_or_default(),
                image_url: a
                    .get("mediaAssets")
                    .and_then(Value::as_array)
                    .and_then(|m| {
                        m.iter()
                            .find(|m| m.get("type").and_then(Value::as_str) == Some("Icon"))
                            .or_else(|| m.first())
                    })
                    .and_then(|m| library_image_url(m.get("url"))),
                is_secret: a.get("isSecret").and_then(Value::as_bool).unwrap_or(false),
                is_unlocked,
                gamerscore: a
                    .get("rewards")
                    .and_then(Value::as_array)
                    .and_then(|r| {
                        r.iter()
                            .find(|r| r.get("type").and_then(Value::as_str) == Some("Gamerscore"))
                    })
                    .and_then(|r| count(r.get("value"))),
                progress_percentage,
                requirements,
                unlocked_at: if is_unlocked {
                    library_string(p.and_then(|p| p.get("timeUnlocked")))
                } else {
                    None
                },
            })
        })
        .collect();
    Ok(LibraryPage {
        items,
        continuation: library_continuation(v),
    })
}
pub fn parse_people(v: &Value) -> Result<Vec<LibraryPerson>, WebApiError> {
    Ok(rows(v, "people")?
        .iter()
        .filter_map(|p| {
            let active = p
                .get("presenceDetails")
                .and_then(Value::as_array)
                .and_then(|ds| {
                    ds.iter().find(|d| {
                        d.get("IsGame").and_then(Value::as_bool) == Some(true)
                            && d.get("State").and_then(Value::as_str) == Some("Active")
                    })
                });
            let gamertag = library_string(p.get("gamertag"))
                .or_else(|| library_string(p.get("uniqueModernGamertag")))
                .or_else(|| library_string(p.get("modernGamertag")))
                .unwrap_or_default();
            Some(LibraryPerson {
                xuid: library_string(p.get("xuid"))?,
                display_name: library_string(p.get("displayName"))
                    .unwrap_or_else(|| gamertag.clone()),
                gamertag,
                avatar_url: library_image_url(p.get("displayPicRaw")),
                gamerscore: library_string(p.get("gamerScore")),
                presence_state: library_string(p.get("presenceState")),
                presence_text: library_string(p.get("presenceText")),
                current_title_id: active.and_then(|d| library_string(d.get("TitleId"))),
                is_followed_by_caller: p.get("isFollowedByCaller").and_then(Value::as_bool),
                is_following_caller: p.get("isFollowingCaller").and_then(Value::as_bool),
            })
        })
        .collect())
}
fn console_rows(v: &Value) -> Result<&Vec<Value>, WebApiError> {
    crate::smartglass_api::validate_remote_management_response(v)?;
    rows(v, "result")
}
pub fn parse_installed_games(v: &Value) -> Result<Vec<InstalledGame>, WebApiError> {
    let mut seen = HashSet::new();
    Ok(console_rows(v)?
        .iter()
        .filter_map(|g| {
            let is_game = g.get("isGame").and_then(Value::as_bool) == Some(true)
                || g.get("contentType").and_then(Value::as_str) == Some("Game");
            if !is_game
                || library_string(g.get("parentId")).is_some()
                || g.get("contentType").and_then(Value::as_str) == Some("Dlc")
            {
                return None;
            }
            let id = library_string(g.get("instanceId"))
                .or_else(|| library_string(g.get("uniqueId")))?;
            if !seen.insert(id.clone()) {
                return None;
            }
            Some(InstalledGame {
                id,
                xbox_title_id: library_string(g.get("titleId")).filter(|s| s != "0"),
                product_id: library_string(g.get("oneStoreProductId")),
                name: library_string(g.get("name"))?,
                artwork_url: None,
                storage_device_id: library_string(g.get("storageDeviceId")),
                size_bytes: library_u64(g.get("sizeInBytes")),
                installed_at: library_string(g.get("installTime")),
                last_played_at: library_string(g.get("lastActiveTime")),
            })
        })
        .collect())
}
pub fn parse_console_storage(v: &Value) -> Result<Vec<ConsoleStorage>, WebApiError> {
    parse_storage_items(console_rows(v)?)
}

pub fn parse_console_storage_for_device(
    v: &Value,
    console_id: &str,
) -> Result<Vec<ConsoleStorage>, WebApiError> {
    let device = console_rows(v)?
        .iter()
        .find(|device| {
            ["id", "deviceId", "serverId"]
                .iter()
                .any(|key| library_string(device.get(key)).as_deref() == Some(console_id.trim()))
        })
        .ok_or_else(|| WebApiError::parse("invalid console storage identity"))?;
    let items = device
        .get("storageDevices")
        .and_then(Value::as_array)
        .ok_or_else(|| WebApiError::parse("missing storageDevices array"))?;
    parse_storage_items(items)
}

fn parse_storage_items(items: &[Value]) -> Result<Vec<ConsoleStorage>, WebApiError> {
    items
        .iter()
        .map(|s| {
            Ok(ConsoleStorage {
                id: library_string(s.get("storageDeviceId").or_else(|| s.get("id")))
                    .ok_or_else(|| WebApiError::parse("invalid console storage response"))?,
                name: library_string(s.get("storageDeviceName").or_else(|| s.get("name")))
                    .unwrap_or_else(|| "Xbox 存储".into()),
                free_bytes: library_u64(s.get("freeSpaceBytes").or_else(|| s.get("freeBytes"))),
                total_bytes: library_u64(s.get("totalSpaceBytes").or_else(|| s.get("totalBytes"))),
            })
        })
        .collect()
}
pub fn parse_media_page(
    v: &Value,
    kind: MediaKind,
) -> Result<LibraryPage<LibraryMedia>, WebApiError> {
    let items = rows(v, "values")?
        .iter()
        .filter_map(|m| {
            let locators = m.get("contentLocators").and_then(Value::as_array);
            let thumbnail = locators.and_then(|ls| {
                ls.iter().find(|l| {
                    library_string(l.get("locatorType"))
                        .is_some_and(|s| s.to_lowercase().contains("thumbnail"))
                })
            });
            let original = locators.and_then(|ls| {
                ls.iter().find(|l| {
                    library_string(l.get("locatorType"))
                        .is_some_and(|s| !s.to_lowercase().contains("thumbnail"))
                })
            });
            let url = original.and_then(|l| library_image_url(l.get("uri")));
            Some(LibraryMedia {
                id: library_string(m.get("contentId"))?,
                kind,
                xbox_title_id: library_string(m.get("titleId")),
                title_name: library_string(m.get("titleName")).unwrap_or_else(|| "Xbox".into()),
                captured_at: library_string(
                    m.get("captureDate")
                        .or_else(|| m.get("uploadDate"))
                        .or_else(|| m.get("dateUploaded")),
                ),
                duration_seconds: library_u64(m.get("durationInSeconds")),
                thumbnail_url: thumbnail
                    .and_then(|l| library_image_url(l.get("uri")))
                    .or_else(|| {
                        if kind == MediaKind::Screenshot {
                            url.clone()
                        } else {
                            None
                        }
                    }),
                url,
                expires_at: original
                    .and_then(|l| library_string(l.get("expiration")))
                    .or_else(|| library_string(m.get("expirationDate"))),
            })
        })
        .collect();
    Ok(LibraryPage {
        items,
        continuation: library_continuation(v),
    })
}
pub fn validate_minutes_played_response(v: &Value) -> Result<(), WebApiError> {
    fn valid_collections(v: &Value, depth: u8) -> bool {
        if depth > 8 || !v.is_object() {
            return false;
        }
        ["groups", "statlistscollection", "stats"]
            .iter()
            .all(|key| match v.get(key) {
                None => true,
                Some(Value::Array(items)) => {
                    items.iter().all(|item| valid_collections(item, depth + 1))
                }
                Some(_) => false,
            })
    }
    let root = library_root(v);
    let has_collection = ["groups", "statlistscollection", "stats"]
        .iter()
        .any(|key| root.get(key).is_some());
    if has_collection && valid_collections(root, 0) {
        Ok(())
    } else {
        Err(WebApiError::parse("invalid statistics response"))
    }
}

pub fn parse_minutes_played(v: &Value, title_id: &str) -> Option<u64> {
    fn visit(v: &Value, title: &str, inherited: Option<&str>, depth: u8) -> Option<u64> {
        if depth > 8 {
            return None;
        }
        let own_id = library_string(v.get("titleId").or_else(|| v.get("titleid")));
        let id = own_id.as_deref().or(inherited);
        if v.get("name")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("MinutesPlayed"))
            && id == Some(title)
        {
            return library_u64(v.get("value"));
        }
        for key in ["groups", "statlistscollection", "stats"] {
            if let Some(items) = v.get(key).and_then(Value::as_array) {
                if let Some(n) = items.iter().find_map(|i| visit(i, title, id, depth + 1)) {
                    return Some(n);
                }
            }
        }
        None
    }
    visit(library_root(v), title_id, None, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn numeric_values_accept_integral_service_floats_and_reject_invalid_capacity() {
        for value in [json!(42), json!(42.0), json!("42"), json!(" 42.0 ")] {
            assert_eq!(library_u64(Some(&value)), Some(42));
        }
        for value in [
            json!(-1),
            json!(1.5),
            json!("NaN"),
            json!("inf"),
            json!("18446744073709551616"),
            json!(null),
        ] {
            assert_eq!(library_u64(Some(&value)), None);
        }
        assert_eq!(library_u64(Some(&json!(u64::MAX))), Some(u64::MAX));
        assert_eq!(
            library_u64(Some(&json!(u64::MAX.to_string()))),
            Some(u64::MAX)
        );
        let storage = parse_console_storage(&json!({"status":{"errorCode":"OK"},"result":[{
            "storageDeviceId":"disk", "freeSpaceBytes":42.0, "totalSpaceBytes":100.0
        }]}))
        .unwrap();
        assert_eq!(storage[0].free_bytes, Some(42));
        assert_eq!(storage[0].total_bytes, Some(100));
    }

    #[test]
    fn embedded_console_storage_matches_identity_and_preserves_capacity() {
        let payload = json!({"data":{"status":{"errorCode":"OK"},"result":[
            {"id":"other","storageDevices":[{"storageDeviceId":"wrong-disk"}]},
            {"id":"target","storageDevices":[{"storageDeviceId":"disk","storageDeviceName":"Internal","freeSpaceBytes":42.0,"totalSpaceBytes":100.0}]}
        ]}});
        let storage = parse_console_storage_for_device(&payload, " target ").unwrap();
        assert_eq!(storage.len(), 1);
        assert_eq!(storage[0].id, "disk");
        assert_eq!(storage[0].free_bytes, Some(42));
        assert_eq!(storage[0].total_bytes, Some(100));
        assert!(parse_console_storage_for_device(&payload, "missing").is_err());
        assert!(
            parse_console_storage_for_device(&json!({"result":[{"id":"target"}]}), "target")
                .is_err()
        );
        assert!(parse_console_storage_for_device(
            &json!({"result":[{"deviceId":"target","storageDevices":[]}]}),
            "target"
        )
        .unwrap()
        .is_empty());
        assert!(parse_console_storage(&json!({"result":[{"freeSpaceBytes":42}]})).is_err());
    }

    #[test]
    fn statistics_schema_preserves_empty_data_and_rejects_invalid_responses() {
        for payload in [
            json!({"statlistscollection":[]}),
            json!({"groups":[{"titleId":"1","statlistscollection":[{"stats":[]}]}]}),
            json!({"data":{"statlistscollection":[{"stats":[{"name":"MinutesPlayed","titleid":"1","value":"42"}]}]}}),
        ] {
            assert!(validate_minutes_played_response(&payload).is_ok());
        }
        for payload in [
            json!({}),
            json!({"stats":null}),
            json!({"stats":[null]}),
            json!({"statlistscollection":[{"stats":"invalid"}]}),
        ] {
            assert_eq!(
                validate_minutes_played_response(&payload)
                    .unwrap_err()
                    .safe_summary()["protocolErrorCode"],
                "invalidStatistics"
            );
        }
    }

    #[test]
    fn first_party_http_artwork_uses_verified_https_and_safe_urls() {
        for (source, expected) in [
            (
                "http://images-eds-ssl.xboxlive.com/image?x=1",
                "https://images-eds-ssl.xboxlive.com/image?x=1",
            ),
            (
                "http://store-images.s-microsoft.com/image/apps.1",
                "https://store-images.s-microsoft.com/image/apps.1",
            ),
            (
                "//store-images.s-microsoft.com/image/apps.1",
                "https://store-images.s-microsoft.com/image/apps.1",
            ),
            (
                "https://example.com/signed.jpg?sig=opaque%2Fvalue",
                "https://example.com/signed.jpg?sig=opaque%2Fvalue",
            ),
        ] {
            assert_eq!(
                library_image_url(Some(&json!(source))).as_deref(),
                Some(expected)
            );
        }
        for source in [
            "http://example.com/image",
            "http://evil-xboxlive.com/image",
            "https://user:password@example.com/image",
            "data:text/html,content",
            "invalid",
        ] {
            assert!(library_image_url(Some(&json!(source))).is_none());
        }
        let page = parse_game_history(&json!({"titles":[{"titleId":"1","name":"Game","images":[{"type":"Tile","url":"http://images-eds-ssl.xboxlive.com/tile"}]}]})).unwrap();
        assert_eq!(
            page.items[0].artwork_url.as_deref(),
            Some("https://images-eds-ssl.xboxlive.com/tile")
        );
    }

    #[test]
    fn history_distinguishes_unplayed_achievement_summary_from_missing_source() {
        let page = parse_game_history(&json!({"titles":[
            {"titleId":"1","name":"Available","achievement":{"sourceVersion":3,"currentAchievements":0,"totalAchievements":46,"currentGamerscore":0,"totalGamerscore":1180}},
            {"titleId":"2","name":"Missing","achievement":{"sourceVersion":0,"currentAchievements":0,"totalAchievements":0,"currentGamerscore":0,"totalGamerscore":0}}
        ]})).unwrap();
        assert_eq!(
            page.items[0].progress.as_ref().unwrap().total_count,
            Some(46)
        );
        assert_eq!(
            page.items[0].progress.as_ref().unwrap().unlocked_count,
            Some(0)
        );
        assert!(page.items[1].progress.is_none());
    }

    #[test]
    fn playtime_batch_matches_decimal_title_ids_and_keeps_missing_values() {
        let payload = json!({"statlistscollection":[{"stats":[
            {"titleid":"4294967295","name":"MinutesPlayed","value":"79"},
            {"titleid":1292135258,"name":"MinutesPlayed","value":"0"},
            {"name":"MinutesPlayed","value":"999"}
        ]}]});
        assert_eq!(parse_minutes_played(&payload, "4294967295"), Some(79));
        assert_eq!(parse_minutes_played(&payload, "1292135258"), Some(0));
        assert_eq!(parse_minutes_played(&payload, "100"), None);
    }
    #[test]
    fn history_preserves_missing_progress_and_large_title_id() {
        let page = parse_game_history(&json!({"titles":[{"titleId":4294967295u64,"name":"Game","type":"Game","achievement":{"currentAchievements":0}}, {"titleId":4294967295u64,"name":"Duplicate"},{"titleId":"1","name":"Video","type":"App"}],"pagingInfo":{"continuationToken":"next/+"}})).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].xbox_title_id, "4294967295");
        assert_eq!(
            page.items[0].progress.as_ref().unwrap().unlocked_count,
            Some(0)
        );
        assert_eq!(page.items[0].progress.as_ref().unwrap().total_count, None);
        assert_eq!(page.continuation.as_deref(), Some("next/+"));
    }
    #[test]
    fn achievement_identity_and_compound_requirements_are_preserved() {
        let page = parse_achievement_page(&json!({"achievements":[{"id":"1","serviceConfigId":"a","progression":{"requirements":[{"id":"r1","current":"4","target":"8"},{"id":"r2","current":"1","target":"3"}]}},{"id":"1","serviceConfigId":"b","progressState":"Achieved"},{"id":"1","serviceConfigId":"a"}]})).unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].requirements.len(), 2);
        assert_eq!(page.items[0].progress_percentage, None);
        assert_eq!(page.items[1].progress_percentage, Some(100.0));
    }
    #[test]
    fn single_requirement_handles_zero_and_invalid_target() {
        let page = parse_achievement_page(&json!({"achievements":[{"id":"1","progression":{"requirements":[{"current":"0","target":"10"}]}},{"id":"2","progression":{"requirements":[{"current":"5","target":"0"}]}}]})).unwrap();
        assert_eq!(page.items[0].progress_percentage, Some(0.0));
        assert_eq!(page.items[1].progress_percentage, None);
    }
    #[test]
    fn installed_list_counts_top_level_games_and_keeps_identity() {
        let page = parse_installed_games(&json!({"status":{"errorCode":"0"},"result":[{"instanceId":"base","isGame":true,"titleId":99,"name":"Base","oneStoreProductId":"STORE","storageDeviceId":"disk","sizeInBytes":0},{"instanceId":"dlc","isGame":true,"contentType":"Dlc","name":"DLC"},{"instanceId":"child","isGame":true,"parentId":"base","name":"Child"},{"instanceId":"app","isGame":false,"name":"App"}]})).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, "base");
        assert_eq!(page[0].xbox_title_id.as_deref(), Some("99"));
        assert_eq!(page[0].size_bytes, Some(0));
        assert!(parse_installed_games(&json!({"status":{"errorCode":"403"},"result":[]})).is_err());
        assert!(parse_installed_games(&json!({"unexpected":[]})).is_err());
    }
    #[test]
    fn people_keep_active_game_and_relationship_semantics() {
        let people = parse_people(&json!({"people":[{"xuid":"2533274799","gamertag":"Player","presenceState":"Online","isFollowingCaller":false,"presenceDetails":[{"IsGame":true,"State":"LastSeen","TitleId":"1"},{"IsGame":true,"State":"Active","TitleId":"2"}]}]})).unwrap();
        assert_eq!(people[0].current_title_id.as_deref(), Some("2"));
        assert_eq!(people[0].is_following_caller, Some(false));
        assert_eq!(people[0].is_followed_by_caller, None);
        let modern = parse_people(&json!({"people":[{"xuid":"1","uniqueModernGamertag":"Player#1234","modernGamertag":"Player"}]})).unwrap();
        assert_eq!(modern[0].gamertag, "Player#1234");
        assert_eq!(modern[0].display_name, "Player#1234");
    }
    #[test]
    fn media_preserves_original_thumbnail_and_expiry() {
        let page = parse_media_page(&json!({"values":[{"contentId":"m","titleId":99,"titleName":"Game","contentLocators":[{"locatorType":"Thumbnail_Large","uri":"https://example.com/thumb.jpg"},{"locatorType":"Download","uri":"https://example.com/clip.mp4","expiration":"2026-11-06T00:00:00Z"}],"durationInSeconds":15}],"pagingInfo":{"continuationToken":"next"}}),MediaKind::Clip).unwrap();
        assert_eq!(
            page.items[0].url.as_deref(),
            Some("https://example.com/clip.mp4")
        );
        assert_eq!(
            page.items[0].thumbnail_url.as_deref(),
            Some("https://example.com/thumb.jpg")
        );
        assert_eq!(
            page.items[0].expires_at.as_deref(),
            Some("2026-11-06T00:00:00Z")
        );
        assert_eq!(page.continuation.as_deref(), Some("next"));
    }
    #[test]
    fn playtime_distinguishes_zero_missing_and_other_title() {
        assert_eq!(
            parse_minutes_played(
                &json!({"groups":[{"titleId":"1","statlistscollection":[{"stats":[{"name":"MinutesPlayed","value":"0"}]}]}]}),
                "1"
            ),
            Some(0)
        );
        assert_eq!(
            parse_minutes_played(
                &json!({"groups":[{"titleId":"2","statlistscollection":[{"stats":[{"name":"MinutesPlayed","value":"42"}]}]}]}),
                "1"
            ),
            None
        );
        assert_eq!(
            parse_minutes_played(
                &json!({"stats":[{"name":"MinutesPlayed","value":"-3"}]}),
                "1"
            ),
            None
        );
        assert_eq!(parse_minutes_played(&json!({"stats":[]}), "1"), None);
    }
}
