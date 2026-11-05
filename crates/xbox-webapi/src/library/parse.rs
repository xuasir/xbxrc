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
fn number(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}
fn count(v: Option<&Value>) -> Option<u32> {
    number(v).and_then(|n| n.try_into().ok())
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
                                .and_then(|i| library_string(i.get("url")))
                        })
                })
                .or_else(|| library_string(g.get("displayImage")));
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
                    .and_then(|m| library_string(m.get("url"))),
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
            Some(LibraryPerson {
                xuid: library_string(p.get("xuid"))?,
                gamertag: library_string(p.get("gamertag")).unwrap_or_default(),
                display_name: library_string(p.get("displayName"))
                    .or_else(|| library_string(p.get("gamertag")))
                    .unwrap_or_default(),
                avatar_url: library_string(p.get("displayPicRaw")),
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
    let root = library_root(v);
    if let Some(code) = library_string(root.get("status").and_then(|s| s.get("errorCode"))) {
        if code != "0" && !code.eq_ignore_ascii_case("success") {
            return Err(WebApiError::parse("主机远程管理查询暂不可用"));
        }
    }
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
                size_bytes: number(g.get("sizeInBytes")),
                installed_at: library_string(g.get("installTime")),
                last_played_at: library_string(g.get("lastActiveTime")),
            })
        })
        .collect())
}
pub fn parse_console_storage(v: &Value) -> Result<Vec<ConsoleStorage>, WebApiError> {
    Ok(console_rows(v)?
        .iter()
        .filter_map(|s| {
            Some(ConsoleStorage {
                id: library_string(s.get("storageDeviceId"))?,
                name: library_string(s.get("storageDeviceName"))
                    .unwrap_or_else(|| "Xbox 存储".into()),
                free_bytes: number(s.get("freeSpaceBytes")),
                total_bytes: number(s.get("totalSpaceBytes")),
            })
        })
        .collect())
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
            let url = original.and_then(|l| library_string(l.get("uri")));
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
                duration_seconds: number(m.get("durationInSeconds")),
                thumbnail_url: thumbnail
                    .and_then(|l| library_string(l.get("uri")))
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
pub fn parse_minutes_played(v: &Value, title_id: &str) -> Option<u64> {
    fn visit(v: &Value, title: &str, inherited: Option<&str>, depth: u8) -> Option<u64> {
        if depth > 8 {
            return None;
        }
        let id = v
            .get("titleId")
            .or_else(|| v.get("titleid"))
            .and_then(Value::as_str)
            .or(inherited);
        if v.get("name")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("MinutesPlayed"))
            && id.is_none_or(|id| id == title)
        {
            return number(v.get("value"));
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
