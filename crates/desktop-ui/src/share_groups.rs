//! Priority-ordered capture groups for launcher-to-game handoff and Share Picker grouping.
use p2pshare_media::{CaptureSource, SourceKind, config::ShareGroup};
use p2pshare_platform_windows::{is_window_minimized, process_executable_name, usable_capture_size};
use std::{cmp::Reverse, collections::{HashMap, HashSet}};

const GROUP_ID_PREFIX: u64 = 0x8000_0000_0000_0000;
const MAX_GROUPS: usize = 64;

pub fn group_source_id(index: usize) -> u64 {
    GROUP_ID_PREFIX + index as u64
}

pub fn group_index_from_id(id: u64) -> Option<usize> {
    let offset = id.checked_sub(GROUP_ID_PREFIX)?;
    (offset < MAX_GROUPS as u64).then_some(offset as usize)
}

#[derive(Clone, Debug)]
pub struct WindowEntry {
    pub source: CaptureSource,
    pub exe: Option<String>,
    pub minimized: bool,
}

pub fn collect_window_entries(raw_sources: &[CaptureSource]) -> Vec<WindowEntry> {
    raw_sources
        .iter()
        .filter(|s| s.kind == SourceKind::Window)
        .map(|s| WindowEntry {
            source: s.clone(),
            exe: s.process_id.and_then(process_executable_name),
            minimized: is_window_minimized(s.id),
        })
        .collect()
}

pub fn running_app_entries(windows: &[WindowEntry]) -> Vec<(String, String)> {
    let mut best: HashMap<String, ((bool, bool, u64), &WindowEntry)> = HashMap::new();
    for w in windows {
        let Some(exe) = &w.exe else { continue };
        if exe.is_empty() {
            continue;
        }
        let area = u64::from(w.source.size.width) * u64::from(w.source.size.height);
        let score = (!w.minimized, !w.source.title.trim().is_empty(), area);
        let key = exe.to_ascii_lowercase();
        best.entry(key)
            .and_modify(|existing| {
                if score > existing.0 {
                    *existing = (score, w);
                }
            })
            .or_insert((score, w));
    }
    let mut entries: Vec<(String, String)> = best
        .into_values()
        .filter_map(|(_, w)| {
            w.exe
                .as_ref()
                .map(|exe| (exe.clone(), w.source.title.trim().to_owned()))
        })
        .collect();
    entries.sort_by_cached_key(|(exe, _)| exe.to_ascii_lowercase());
    entries
}

pub fn process_priority_rank(group: &ShareGroup, exe: Option<&str>) -> Option<usize> {
    let exe = exe?;
    group
        .processes
        .iter()
        .position(|p| p.eq_ignore_ascii_case(exe))
}

pub fn pick_best_window<'a>(
    group: &ShareGroup,
    windows: &'a [WindowEntry],
) -> Option<&'a WindowEntry> {
    windows
        .iter()
        .filter_map(|w| {
            let rank = process_priority_rank(group, w.exe.as_deref())?;
            let area = u64::from(w.source.size.width) * u64::from(w.source.size.height);
            Some((
                (
                    rank,
                    w.minimized,
                    !usable_capture_size(w.source.size),
                    Reverse(area),
                ),
                w,
            ))
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, w)| w)
}

pub fn should_switch_group_window(
    group: &ShareGroup,
    current_id: Option<u64>,
    current_exe: Option<&str>,
    current_alive: bool,
    best: &WindowEntry,
) -> bool {
    let Some(current_id) = current_id else {
        return true;
    };
    if !current_alive {
        return true;
    }
    if current_id == best.source.id {
        return false;
    }
    let Some(cur_rank) = process_priority_rank(group, current_exe) else {
        return true;
    };
    let Some(best_rank) = process_priority_rank(group, best.exe.as_deref()) else {
        return false;
    };
    // Only pre-empt an active window when a strictly higher-priority executable opens.
    best_rank < cur_rank
}

#[derive(Clone, Debug)]
pub struct PickerItem {
    pub source: CaptureSource,
    pub subtitle: String,
    pub grouped: bool,
    pub thumbnail_id: u64,
}

pub fn source_subtitle(
    source: &CaptureSource,
    exe: Option<&str>,
    minimized: bool,
) -> String {
    match source.kind {
        SourceKind::Monitor => {
            if !source.size.is_empty() {
                format!(
                    "{} · {}×{}",
                    p2pshare_media::tr!("Display"),
                    source.size.width,
                    source.size.height
                )
            } else {
                p2pshare_media::tr!("Display").into()
            }
        }
        SourceKind::Window => {
            let base = exe
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| p2pshare_media::tr!("Application"));
            if minimized {
                format!("{base} · {}", p2pshare_media::tr!("Minimized"))
            } else if !source.size.is_empty() {
                format!("{base} · {}×{}", source.size.width, source.size.height)
            } else {
                base.into()
            }
        }
    }
}

pub fn build_picker_items(
    raw_sources: &[CaptureSource],
    windows: &[WindowEntry],
    groups: &[ShareGroup],
) -> (Vec<PickerItem>, Vec<CaptureSource>) {
    let mut items = Vec::new();
    let mut thumb_sources = Vec::new();
    let mut seen_thumbs = HashSet::new();
    let mut merged_exes = HashSet::new();

    for (gi, group) in groups.iter().enumerate() {
        let Some(best) = pick_best_window(group, windows) else {
            continue;
        };
        if group.merge_in_picker {
            for proc_name in &group.processes {
                merged_exes.insert(proc_name.to_ascii_lowercase());
            }
        }
        items.push(PickerItem {
            source: CaptureSource {
                id: group_source_id(gi),
                kind: SourceKind::Window,
                title: group.name.clone(),
                size: best.source.size,
                process_id: best.source.process_id,
            },
            subtitle: format!(
                "{} · {}",
                p2pshare_media::tr!("Group"),
                group.processes.join(" → ")
            ),
            grouped: true,
            thumbnail_id: best.source.id,
        });
        if seen_thumbs.insert(best.source.id) {
            thumb_sources.push(best.source.clone());
        }
    }

    let window_by_id: std::collections::HashMap<u64, &WindowEntry> =
        windows.iter().map(|w| (w.source.id, w)).collect();

    for raw in raw_sources {
        let win = if raw.kind == SourceKind::Window {
            window_by_id.get(&raw.id).copied()
        } else {
            None
        };
        let exe = win.and_then(|w| w.exe.as_deref());
        if raw.kind == SourceKind::Window
            && exe.is_some_and(|name| merged_exes.contains(&name.to_ascii_lowercase()))
        {
            continue;
        }
        let minimized = win.is_some_and(|w| w.minimized);
        items.push(PickerItem {
            source: raw.clone(),
            subtitle: source_subtitle(raw, exe, minimized),
            grouped: false,
            thumbnail_id: raw.id,
        });
        if seen_thumbs.insert(raw.id) {
            thumb_sources.push(raw.clone());
        }
    }

    (items, thumb_sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use p2pshare_media::Dimensions;

    fn win(id: u64, title: &str, exe: &str, w: u32, h: u32, minimized: bool) -> WindowEntry {
        WindowEntry {
            source: CaptureSource {
                id,
                kind: SourceKind::Window,
                title: title.into(),
                size: Dimensions::new(w, h),
                process_id: Some(id as u32),
            },
            exe: Some(exe.into()),
            minimized,
        }
    }

    #[test]
    fn group_id_round_trips_and_never_collides_with_low_handles() {
        assert_eq!(group_index_from_id(0x0012_3456), None);
        let id = group_source_id(3);
        assert_eq!(group_index_from_id(id), Some(3));
    }

    #[test]
    fn league_game_promotes_over_lobby_and_falls_back_on_exit() {
        let group = &p2pshare_media::config::default_share_groups()[0];
        let lobby = win(10, "League of Legends", "LeagueClientUx.exe", 1600, 900, false);
        let lobby_min = win(10, "League of Legends", "LeagueClientUx.exe", 1600, 900, true);
        let game = win(20, "League of Legends (TM) Client", "League of Legends.exe", 1920, 1080, false);
        let discord = win(30, "Discord", "Discord.exe", 1920, 1032, false);

        // Start in lobby.
        let lobby_only = [discord.clone(), lobby.clone()];
        let picked = pick_best_window(group, &lobby_only).unwrap();
        assert_eq!(picked.source.id, 10);
        assert!(should_switch_group_window(group, None, None, false, picked));

        // Match spawns and lobby minimizes -> promote to game.exe.
        let with_game = [discord.clone(), lobby_min.clone(), game.clone()];
        let picked = pick_best_window(group, &with_game).unwrap();
        assert_eq!(picked.source.id, 20);
        assert!(should_switch_group_window(
            group,
            Some(10),
            Some("LeagueClientUx.exe"),
            true,
            picked
        ));

        // Same game window stays selected (no switch).
        assert!(!should_switch_group_window(
            group,
            Some(20),
            Some("League of Legends.exe"),
            true,
            picked
        ));

        // Match exits -> fall back to lobby.
        let after_exit = [discord, lobby];
        let picked = pick_best_window(group, &after_exit).unwrap();
        assert_eq!(picked.source.id, 10);
        assert!(should_switch_group_window(
            group,
            Some(20),
            Some("League of Legends.exe"),
            false,
            picked
        ));
    }

    #[test]
    fn equal_priority_windows_do_not_ping_pong_while_current_is_alive() {
        let group = ShareGroup {
            name: "Multi".into(),
            merge_in_picker: true,
            processes: vec!["app.exe".into()],
        };
        let w1 = win(1, "Window 1", "app.exe", 1280, 720, false);
        let w2 = win(2, "Window 2", "app.exe", 1920, 1080, false);
        let both = [w1.clone(), w2.clone()];
        let best = pick_best_window(&group, &both).unwrap();
        assert_eq!(best.source.id, 2);
        assert!(!should_switch_group_window(
            &group,
            Some(1),
            Some("app.exe"),
            true,
            best
        ));
        assert!(should_switch_group_window(
            &group,
            Some(1),
            Some("app.exe"),
            false,
            best
        ));
    }

    #[test]
    fn picker_merges_or_keeps_individual_cards_based_on_flag() {
        let mut groups = p2pshare_media::config::default_share_groups();
        let lobby = win(10, "League of Legends", "LeagueClientUx.exe", 1600, 900, false);
        let game = win(20, "League of Legends (TM) Client", "League of Legends.exe", 1920, 1080, false);
        let discord = win(30, "Discord", "Discord.exe", 1920, 1032, false);
        let monitor = CaptureSource {
            id: 1,
            kind: SourceKind::Monitor,
            title: "Monitor \\\\.\\DISPLAY1 (primary)".into(),
            size: Dimensions::new(1920, 1080),
            process_id: None,
        };
        let raw = vec![
            monitor.clone(),
            lobby.source.clone(),
            game.source.clone(),
            discord.source.clone(),
        ];
        let windows = vec![lobby, game, discord];

        let (merged_items, thumbs) = build_picker_items(&raw, &windows, &groups);
        // Expected: League group + Monitor + Discord (individual League windows hidden)
        assert_eq!(merged_items.len(), 3);
        assert!(merged_items[0].grouped);
        assert_eq!(merged_items[0].source.title, "League of Legends");
        assert_eq!(merged_items[0].thumbnail_id, 20);
        assert_eq!(thumbs.len(), 3);

        groups[0].merge_in_picker = false;
        let (unmerged_items, _) = build_picker_items(&raw, &windows, &groups);
        // Expected: League group + Monitor + Lobby + Game + Discord
        assert_eq!(unmerged_items.len(), 5);
    }

    #[test]
    fn running_app_entries_deduplicate_and_prefer_visible_largest_title() {
        let min_win = win(1, "Client (Minimized)", "Discord.exe", 1920, 1080, true);
        let small_win = win(2, "Popout", "discord.exe", 600, 400, false);
        let main_win = win(3, "General - Discord", "Discord.exe", 1600, 900, false);
        let blank_win = win(4, "   ", "obs64.exe", 1280, 720, false);
        let obs_win = win(5, "OBS 30.2", "obs64.exe", 1920, 1080, false);
        let entries = running_app_entries(&[min_win, small_win, main_win, blank_win, obs_win]);
        assert_eq!(
            entries,
            vec![
                ("Discord.exe".to_owned(), "General - Discord".to_owned()),
                ("obs64.exe".to_owned(), "OBS 30.2".to_owned()),
            ]
        );
    }
}
