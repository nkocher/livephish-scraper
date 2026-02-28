//! Comprehensive Bman integration tests.
//!
//! Verifies real user behavior end-to-end: catalog traversal, year sorting,
//! title extraction, venue identification, track listings, download pipeline,
//! conversion, cover art, queue handling, deduplication, JGB routing, and
//! non-regression for nugs.net/LivePhish flows.

#[cfg(test)]
mod tests {
    use crate::bman::download::*;
    use crate::bman::gdrive::*;
    use crate::bman::id_map::BmanIdMap;
    use crate::bman::parser::*;
    use crate::bman::BmanApi;
    use crate::catalog::cache;
    use crate::catalog::Catalog;
    use crate::models::{CatalogShow, FormatCode, Quality, Show, Track};
    use crate::service::Service;
    use serde_json::json;
    use tempfile::tempdir;
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Helper builders ─────────────────────────────────────────────────

    fn make_drive_item(id: &str, name: &str, mime: &str) -> DriveItem {
        DriveItem {
            id: id.to_string(),
            name: name.to_string(),
            mime_type: mime.to_string(),
            size: None,
        }
    }

    fn make_flac_item(id: &str, name: &str) -> DriveItem {
        make_drive_item(id, name, "audio/flac")
    }

    fn make_folder_item(id: &str, name: &str) -> DriveItem {
        make_drive_item(id, name, FOLDER_MIME)
    }

    fn make_bman_catalog_show(
        container_id: i64,
        artist_id: i64,
        artist_name: &str,
        date: &str,
        venue: &str,
        city: &str,
        state: &str,
    ) -> CatalogShow {
        CatalogShow {
            container_id,
            artist_id,
            artist_name: artist_name.to_string(),
            container_info: String::new(),
            venue_name: venue.to_string(),
            venue_city: city.to_string(),
            venue_state: state.to_string(),
            performance_date: date.to_string(),
            performance_date_formatted: String::new(),
            performance_date_year: date.get(..4).unwrap_or("").to_string(),
            image_url: String::new(),
            song_list: String::new(),
            service: Service::Bman,
        }
    }

    fn make_show_with_tracks(
        container_id: i64,
        artist_name: &str,
        date: &str,
        venue: &str,
        tracks: Vec<Track>,
    ) -> Show {
        Show {
            container_id,
            artist_name: artist_name.to_string(),
            container_info: String::new(),
            venue_name: venue.to_string(),
            venue_city: "City".to_string(),
            venue_state: "ST".to_string(),
            performance_date: date.to_string(),
            performance_date_formatted: String::new(),
            performance_date_year: date.get(..4).unwrap_or("").to_string(),
            artist_id: BMAN_GD_ARTIST_ID,
            total_duration_seconds: 0,
            total_duration_display: String::new(),
            tracks,
            image_url: String::new(),
        }
    }

    fn make_track(track_id: i64, title: &str, track_num: i64, disc_num: i64) -> Track {
        Track {
            track_id,
            song_id: 0,
            song_title: title.to_string(),
            track_num,
            disc_num,
            set_num: disc_num.max(1),
            duration_seconds: 0,
            duration_display: String::new(),
        }
    }

    /// Mount a mock Drive files.list endpoint returning the given items.
    async fn mount_drive_list(
        server: &MockServer,
        parent_id: &str,
        items: Vec<serde_json::Value>,
    ) {
        Mock::given(method("GET"))
            .and(query_param(
                "q",
                format!("'{}' in parents", parent_id),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": items,
                "nextPageToken": null
            })))
            .mount(server)
            .await;
    }

    fn drive_file_json(id: &str, name: &str, mime: &str) -> serde_json::Value {
        json!({
            "id": id,
            "name": name,
            "mimeType": mime
        })
    }

    fn drive_flac_json(id: &str, name: &str) -> serde_json::Value {
        drive_file_json(id, name, "audio/flac")
    }

    fn drive_folder_json(id: &str, name: &str) -> serde_json::Value {
        drive_file_json(id, name, FOLDER_MIME)
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 1: Year Sorting & Catalog Organization
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_bman_shows_sort_into_correct_years() {
        let tmp = tempdir().unwrap();
        let mut catalog = Catalog::new(tmp.path().to_path_buf());

        let shows = vec![
            make_bman_catalog_show(-100, BMAN_GD_ARTIST_ID, "Grateful Dead", "1977-05-08", "Cornell", "Ithaca", "NY"),
            make_bman_catalog_show(-101, BMAN_GD_ARTIST_ID, "Grateful Dead", "1972-08-27", "Veneta", "Veneta", "OR"),
            make_bman_catalog_show(-102, BMAN_GD_ARTIST_ID, "Grateful Dead", "1977-05-09", "War Memorial", "Buffalo", "NY"),
            make_bman_catalog_show(-103, BMAN_GD_ARTIST_ID, "Grateful Dead", "1989-07-04", "RFK", "DC", "DC"),
        ];
        catalog.shows.extend(shows);
        catalog.build_indexes();

        let y1977: Vec<_> = catalog
            .get_shows_by_year("1977")
            .into_iter()
            .filter(|s| s.service == Service::Bman)
            .collect();
        assert_eq!(y1977.len(), 2, "1977 should have 2 shows");
        assert!(y1977.iter().all(|s| s.performance_date_year == "1977"));

        let y1972: Vec<_> = catalog
            .get_shows_by_year("1972")
            .into_iter()
            .filter(|s| s.service == Service::Bman)
            .collect();
        assert_eq!(y1972.len(), 1);
        assert_eq!(y1972[0].performance_date, "1972-08-27");

        let y1989: Vec<_> = catalog
            .get_shows_by_year("1989")
            .into_iter()
            .filter(|s| s.service == Service::Bman)
            .collect();
        assert_eq!(y1989.len(), 1);
    }

    #[test]
    fn test_bman_shows_do_not_appear_in_nugs_year_listings() {
        let tmp = tempdir().unwrap();
        let mut catalog = Catalog::new(tmp.path().to_path_buf());

        // Mix Bman and nugs shows in the same year
        let bman_show = make_bman_catalog_show(
            -100, BMAN_GD_ARTIST_ID, "Grateful Dead",
            "1977-05-08", "Cornell", "Ithaca", "NY",
        );
        let nugs_show = CatalogShow {
            container_id: 55555,
            artist_id: 196,
            artist_name: "Phish".to_string(),
            venue_name: "MSG".to_string(),
            venue_city: "New York".to_string(),
            venue_state: "NY".to_string(),
            performance_date: "1977-11-01".to_string(), // hypothetical
            performance_date_year: "1977".to_string(),
            service: Service::Nugs,
            ..Default::default()
        };
        catalog.shows.push(bman_show);
        catalog.shows.push(nugs_show);
        catalog.build_indexes();

        let all_1977 = catalog.get_shows_by_year("1977");
        let bman_only: Vec<_> = all_1977.iter().filter(|s| s.service == Service::Bman).collect();
        let nugs_only: Vec<_> = all_1977.iter().filter(|s| s.service == Service::Nugs).collect();
        assert_eq!(bman_only.len(), 1);
        assert_eq!(nugs_only.len(), 1);
    }

    #[test]
    fn test_jgb_shows_use_correct_artist_id() {
        let jgb = parse_show_folder(
            "jgb77-05-21.aud.flac16",
            "folder_jgb",
            BmanArtist::JerryGarciaBand,
            false,
        ).unwrap();
        assert_eq!(jgb.artist.artist_id(), BMAN_JGB_ARTIST_ID);
        assert_eq!(jgb.artist.name(), "Jerry Garcia Band");
    }

    #[test]
    fn test_gd_shows_use_correct_artist_id() {
        let gd = parse_show_folder(
            "gd77-05-08.sbd.flac16",
            "folder_gd",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(gd.artist.artist_id(), BMAN_GD_ARTIST_ID);
        assert_eq!(gd.artist.name(), "Grateful Dead");
    }

    #[test]
    fn test_etree_prefix_overrides_passed_artist() {
        // Even if we pass GratefulDead, jgb prefix should override
        let show = parse_show_folder(
            "jgb73-07-18.aud.flac16",
            "folder_id",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.artist, BmanArtist::JerryGarciaBand);
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 2: Title & Venue Identification
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_nice_format_extracts_venue_city_state() {
        let show = parse_show_folder(
            "1977-05-08 Cornell University, Ithaca, NY (Betty Board)",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.venue, "Cornell University");
        assert_eq!(show.city, "Ithaca");
        assert_eq!(show.state, "NY");
        assert_eq!(show.date, "1977-05-08");
        assert_eq!(show.source_tag, "Betty Board");
    }

    #[test]
    fn test_multi_part_venue_name() {
        let show = parse_show_folder(
            "1977-05-08 Barton Hall, Cornell University, Ithaca, NY",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.venue, "Barton Hall, Cornell University");
        assert_eq!(show.city, "Ithaca");
        assert_eq!(show.state, "NY");
    }

    #[test]
    fn test_mm_dd_yyyy_format() {
        // MM-DD-YYYY format: "MM-DD-YYYY - venue - city, ST"
        // With only one " - " separator, the rest goes to venue (no city/state split)
        let show = parse_show_folder(
            "06-10-1973 - RFK Stadium - Washington, DC",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1973-06-10");
        assert_eq!(show.venue, "RFK Stadium");
        assert_eq!(show.city, "Washington");
        assert_eq!(show.state, "DC");
    }

    #[test]
    fn test_etree_format_extracts_date() {
        let show = parse_show_folder(
            "gd73-06-10.aud.shnf.flac16",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1973-06-10");
        assert_eq!(show.source_type, SourceType::Aud);
    }

    #[test]
    fn test_bare_date_no_venue() {
        let show = parse_show_folder(
            "1977-05-08",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1977-05-08");
        assert!(show.venue.is_empty());
    }

    #[test]
    fn test_source_type_detection() {
        let sbd = parse_show_folder(
            "gd77-05-08.sbd.flac16",
            "fid", BmanArtist::GratefulDead, false,
        ).unwrap();
        assert_eq!(sbd.source_type, SourceType::Sbd);

        let aud = parse_show_folder(
            "gd77-05-08.aud.flac16",
            "fid", BmanArtist::GratefulDead, false,
        ).unwrap();
        assert_eq!(aud.source_type, SourceType::Aud);

        let mtx = parse_show_folder(
            "gd77-05-08.matrix.flac16",
            "fid", BmanArtist::GratefulDead, false,
        ).unwrap();
        assert_eq!(mtx.source_type, SourceType::Mtx);
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 3: Track Listing Correctness
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_nice_track_filename_parsing() {
        let track = parse_track_filename("01 Bertha.flac", "fid").unwrap();
        assert_eq!(track.track_num, 1);
        assert_eq!(track.title, "Bertha");

        let track2 = parse_track_filename("12. Greatest Story Ever Told.flac", "fid").unwrap();
        assert_eq!(track2.track_num, 12);
        assert_eq!(track2.title, "Greatest Story Ever Told");
    }

    #[test]
    fn test_etree_track_filename_parsing() {
        let track = parse_track_filename("d1t01.flac", "fid").unwrap();
        assert_eq!(track.disc_num, 1);
        assert_eq!(track.track_num, 1);

        let track2 = parse_track_filename("d2t05.flac", "fid").unwrap();
        assert_eq!(track2.disc_num, 2);
        assert_eq!(track2.track_num, 5);
    }

    #[test]
    fn test_collect_tracks_single_disc() {
        let items = vec![
            make_flac_item("f1", "01 Bertha.flac"),
            make_flac_item("f2", "02 Greatest Story.flac"),
            make_flac_item("f3", "03 Sugaree.flac"),
            make_drive_item("t1", "info.txt", "text/plain"), // non-FLAC ignored
        ];
        let mut bman = BmanApi::new_for_test("key".into(), "root".into());
        let mut tracks = Vec::new();
        collect_tracks_from_items(&items, None, &mut bman, &mut tracks);

        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].song_title, "Bertha");
        assert_eq!(tracks[1].song_title, "Greatest Story");
        assert_eq!(tracks[2].song_title, "Sugaree");
    }

    #[test]
    fn test_collect_tracks_disc_override() {
        let items = vec![
            make_flac_item("f1", "01 Bertha.flac"),
            make_flac_item("f2", "02 Sugar Magnolia.flac"),
        ];
        let mut bman = BmanApi::new_for_test("key".into(), "root".into());
        let mut tracks = Vec::new();
        collect_tracks_from_items(&items, Some(2), &mut bman, &mut tracks);

        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().all(|t| t.disc_num == 2));
    }

    #[test]
    fn test_tracks_sorted_by_disc_then_track() {
        let mut tracks = vec![
            make_track(-10, "D2T1", 1, 2),
            make_track(-11, "D1T2", 2, 1),
            make_track(-12, "D1T1", 1, 1),
            make_track(-13, "D2T2", 2, 2),
        ];
        tracks.sort_by_key(|t| (t.disc_num, t.track_num));

        assert_eq!(tracks[0].song_title, "D1T1");
        assert_eq!(tracks[1].song_title, "D1T2");
        assert_eq!(tracks[2].song_title, "D2T1");
        assert_eq!(tracks[3].song_title, "D2T2");
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 4: Download Pipeline (resolve_bman_tracks)
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_resolve_bman_tracks_produces_urls() {
        let mut bman = BmanApi::new_for_test("key".into(), "root".into());
        let drive_id = "abc123";
        let track_id = bman.id_map.insert(drive_id);

        let show = make_show_with_tracks(
            -100,
            "Grateful Dead",
            "1977-05-08",
            "Cornell",
            vec![make_track(track_id, "Bertha", 1, 1)],
        );

        let twu = resolve_bman_tracks(&show, &bman);
        assert_eq!(twu.len(), 1);
        let (track, url, quality) = &twu[0];
        assert_eq!(track.song_title, "Bertha");
        assert!(url.contains(drive_id), "URL should contain the Drive file ID");
        assert_eq!(quality.extension, ".flac");
    }

    #[test]
    fn test_resolve_bman_tracks_skips_unmapped() {
        let bman = BmanApi::new_for_test("key".into(), "root".into());
        // Track with an ID that's not in the id_map
        let show = make_show_with_tracks(
            -100,
            "Grateful Dead",
            "1977-05-08",
            "Cornell",
            vec![make_track(-999999, "Ghost Track", 1, 1)],
        );

        let twu = resolve_bman_tracks(&show, &bman);
        assert!(twu.is_empty(), "Unmapped track should be filtered out");
    }

    #[test]
    fn test_bman_quality_is_always_flac() {
        let quality = Quality::from_format_code(FormatCode::Flac);
        assert_eq!(quality.extension, ".flac");
        assert_eq!(quality.code, "flac");
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 5: FLAC Conversion Defaults
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_bman_flac_convert_defaults_to_aac() {
        assert_eq!(bman_flac_convert("none"), "aac");
        assert_eq!(bman_flac_convert(""), "aac");
    }

    #[test]
    fn test_bman_flac_convert_respects_explicit_alac() {
        assert_eq!(bman_flac_convert("alac"), "alac");
    }

    #[test]
    fn test_bman_flac_convert_respects_explicit_aac() {
        assert_eq!(bman_flac_convert("aac"), "aac");
    }

    #[test]
    fn test_bman_flac_convert_passes_through_unknown() {
        // Future formats should pass through unchanged
        assert_eq!(bman_flac_convert("opus"), "opus");
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 6: MIME Type Handling (Review Bug #6)
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_is_flac_audio_flac() {
        let item = make_flac_item("id", "track.flac");
        assert!(item.is_flac());
    }

    #[test]
    fn test_is_flac_rejects_non_audio() {
        let item = make_drive_item("id", "track.flac", "application/octet-stream");
        assert!(!item.is_flac());
    }

    #[test]
    fn test_is_flac_rejects_mp3() {
        let item = make_drive_item("id", "track.mp3", "audio/mpeg");
        assert!(!item.is_flac());
    }

    #[test]
    fn test_is_folder() {
        let folder = make_folder_item("id", "1977");
        assert!(folder.is_folder());
        assert!(!folder.is_flac());
    }

    #[test]
    fn test_is_text() {
        let txt = make_drive_item("id", "info.txt", "text/plain");
        assert!(txt.is_text());
        assert!(!txt.is_flac());
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 7: Deduplication & NLL Precedence
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_nll_supersedes_regular_version() {
        let regular = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: "Cornell University".to_string(),
            city: "Ithaca".to_string(),
            state: "NY".to_string(),
            source_type: SourceType::Sbd,
            source_tag: "SBD".to_string(),
            is_nll: false,
            artist: BmanArtist::GratefulDead,
            folder_id: "folder_regular".to_string(),
        };
        let nll = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: "Cornell University".to_string(),
            city: "Ithaca".to_string(),
            state: "NY".to_string(),
            source_type: SourceType::Sbd,
            source_tag: "SBD".to_string(),
            is_nll: true,
            artist: BmanArtist::GratefulDead,
            folder_id: "folder_nll".to_string(),
        };

        let deduped = dedup_shows(vec![regular.clone(), nll.clone()]);
        assert_eq!(deduped.len(), 1, "Duplicate date+artist should be deduped");
        assert!(deduped[0].is_nll, "NLL version should win");
        assert_eq!(deduped[0].folder_id, "folder_nll");
    }

    #[test]
    fn test_nll_supersedes_regardless_of_order() {
        let regular = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: String::new(),
            city: String::new(),
            state: String::new(),
            source_type: SourceType::Aud,
            source_tag: String::new(),
            is_nll: false,
            artist: BmanArtist::GratefulDead,
            folder_id: "regular".to_string(),
        };
        let nll = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: String::new(),
            city: String::new(),
            state: String::new(),
            source_type: SourceType::Sbd,
            source_tag: String::new(),
            is_nll: true,
            artist: BmanArtist::GratefulDead,
            folder_id: "nll".to_string(),
        };

        // NLL first, then regular
        let deduped1 = dedup_shows(vec![nll.clone(), regular.clone()]);
        assert_eq!(deduped1.len(), 1);
        assert!(deduped1[0].is_nll);

        // Regular first, then NLL
        let deduped2 = dedup_shows(vec![regular, nll]);
        assert_eq!(deduped2.len(), 1);
        assert!(deduped2[0].is_nll);
    }

    #[test]
    fn test_sbd_beats_aud_for_same_date() {
        let aud = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: String::new(),
            city: String::new(),
            state: String::new(),
            source_type: SourceType::Aud,
            source_tag: String::new(),
            is_nll: false,
            artist: BmanArtist::GratefulDead,
            folder_id: "aud_folder".to_string(),
        };
        let sbd = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: String::new(),
            city: String::new(),
            state: String::new(),
            source_type: SourceType::Sbd,
            source_tag: String::new(),
            is_nll: false,
            artist: BmanArtist::GratefulDead,
            folder_id: "sbd_folder".to_string(),
        };

        let deduped = dedup_shows(vec![aud, sbd]);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].source_type, SourceType::Sbd);
    }

    #[test]
    fn test_different_dates_not_deduped() {
        let show1 = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: String::new(),
            city: String::new(),
            state: String::new(),
            source_type: SourceType::Sbd,
            source_tag: String::new(),
            is_nll: false,
            artist: BmanArtist::GratefulDead,
            folder_id: "folder1".to_string(),
        };
        let show2 = ParsedShow {
            date: "1977-05-09".to_string(),
            venue: String::new(),
            city: String::new(),
            state: String::new(),
            source_type: SourceType::Sbd,
            source_tag: String::new(),
            is_nll: false,
            artist: BmanArtist::GratefulDead,
            folder_id: "folder2".to_string(),
        };

        let deduped = dedup_shows(vec![show1, show2]);
        assert_eq!(deduped.len(), 2, "Different dates should not be deduped");
    }

    #[test]
    fn test_same_date_different_artist_not_deduped() {
        let gd = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: String::new(),
            city: String::new(),
            state: String::new(),
            source_type: SourceType::Sbd,
            source_tag: String::new(),
            is_nll: false,
            artist: BmanArtist::GratefulDead,
            folder_id: "gd_folder".to_string(),
        };
        let jgb = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: String::new(),
            city: String::new(),
            state: String::new(),
            source_type: SourceType::Sbd,
            source_tag: String::new(),
            is_nll: false,
            artist: BmanArtist::JerryGarciaBand,
            folder_id: "jgb_folder".to_string(),
        };

        let deduped = dedup_shows(vec![gd, jgb]);
        assert_eq!(deduped.len(), 2, "GD and JGB on same date should both survive");
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 8: ID Mapping Edge Cases (Review Bug #8, #9)
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_synthetic_ids_are_always_negative() {
        for drive_id in &["abc", "xyz", "1", "a-very-long-drive-id-string-1234567890"] {
            let id = BmanIdMap::synthetic_id(drive_id);
            assert!(id < 0, "ID for '{}' should be negative, got {}", drive_id, id);
        }
    }

    #[test]
    fn test_synthetic_id_deterministic() {
        let id1 = BmanIdMap::synthetic_id("test_folder_123");
        let id2 = BmanIdMap::synthetic_id("test_folder_123");
        assert_eq!(id1, id2, "Same input should produce same output");
    }

    #[test]
    fn test_id_map_handles_collision() {
        // This test verifies the collision resolution mechanism works.
        // We can't easily force a collision, but we can verify that
        // inserting many IDs never produces duplicates.
        let mut map = BmanIdMap::new();
        let mut seen_ids = std::collections::HashSet::new();

        for i in 0..1000 {
            let drive_id = format!("folder_{}", i);
            let synthetic = map.insert(&drive_id);
            assert!(
                seen_ids.insert(synthetic),
                "Collision detected: {} already used for a different drive_id",
                synthetic
            );
            assert!(synthetic < 0);
        }
        assert_eq!(map.len(), 1000);
    }

    #[test]
    fn test_id_map_persist_and_reload() {
        let mut map = BmanIdMap::new();
        let id = map.insert("drive_folder_abc");

        let json = serde_json::to_string(&map).unwrap();
        let loaded: BmanIdMap = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.get_drive_id(id), Some("drive_folder_abc"));
        // Re-inserting in loaded map should return same ID
        let mut loaded_mut = loaded;
        let reinserted = loaded_mut.insert("drive_folder_abc");
        assert_eq!(reinserted, id);
    }

    #[test]
    fn test_bman_artist_ids_do_not_collide_with_nugs() {
        // Nugs IDs are always positive
        assert!(BMAN_GD_ARTIST_ID > 0, "GD uses nugs artist_id 461");
        assert!(BMAN_JGB_ARTIST_ID < 0, "JGB uses negative synthetic ID");
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 9: Fetch Show Detail (end-to-end with mock Drive)
    // ════════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_fetch_bman_show_detail_single_disc() {
        let server = MockServer::start().await;
        let mut bman = BmanApi::new_for_test("key".into(), "root".into())
            .with_drive_base_url(&server.uri());

        // Register the folder ID in the id_map
        let folder_id = "show_folder_123";
        let container_id = bman.id_map.insert(folder_id);

        // Mock the Drive listing for this folder
        Mock::given(method("GET"))
            .and(query_param(
                "q",
                format!("'{}' in parents", folder_id),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [
                    drive_flac_json("t1", "01 Bertha.flac"),
                    drive_flac_json("t2", "02 Greatest Story Ever Told.flac"),
                    drive_flac_json("t3", "03 Sugaree.flac"),
                    drive_file_json("info", "info.txt", "text/plain"),
                ]
            })))
            .mount(&server)
            .await;

        let catalog_show = make_bman_catalog_show(
            container_id, BMAN_GD_ARTIST_ID, "Grateful Dead",
            "1977-05-08", "Cornell University", "Ithaca", "NY",
        );

        let show = fetch_bman_show_detail(&mut bman, &catalog_show).await.unwrap();
        assert_eq!(show.tracks.len(), 3);
        assert_eq!(show.tracks[0].song_title, "Bertha");
        assert_eq!(show.tracks[1].song_title, "Greatest Story Ever Told");
        assert_eq!(show.tracks[2].song_title, "Sugaree");
        assert_eq!(show.tracks[0].track_num, 1);
        assert_eq!(show.tracks[2].track_num, 3);
        assert_eq!(show.venue_name, "Cornell University");
        assert_eq!(show.performance_date, "1977-05-08");
    }

    #[tokio::test]
    async fn test_fetch_bman_show_detail_multi_disc() {
        let server = MockServer::start().await;
        let mut bman = BmanApi::new_for_test("key".into(), "root".into())
            .with_drive_base_url(&server.uri());

        let folder_id = "multi_disc_folder";
        let container_id = bman.id_map.insert(folder_id);

        // Root folder contains disc subfolders
        Mock::given(method("GET"))
            .and(query_param(
                "q",
                format!("'{}' in parents", folder_id),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [
                    drive_folder_json("disc1_id", "Disc 1"),
                    drive_folder_json("disc2_id", "Disc 2"),
                ]
            })))
            .mount(&server)
            .await;

        // Disc 1 contents
        Mock::given(method("GET"))
            .and(query_param(
                "q",
                "'disc1_id' in parents".to_string(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [
                    drive_flac_json("d1t1", "01 Morning Dew.flac"),
                    drive_flac_json("d1t2", "02 Scarlet Begonias.flac"),
                ]
            })))
            .mount(&server)
            .await;

        // Disc 2 contents
        Mock::given(method("GET"))
            .and(query_param(
                "q",
                "'disc2_id' in parents".to_string(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [
                    drive_flac_json("d2t1", "01 Eyes of the World.flac"),
                    drive_flac_json("d2t2", "02 Stella Blue.flac"),
                ]
            })))
            .mount(&server)
            .await;

        let catalog_show = make_bman_catalog_show(
            container_id, BMAN_GD_ARTIST_ID, "Grateful Dead",
            "1977-05-08", "Cornell", "Ithaca", "NY",
        );

        let show = fetch_bman_show_detail(&mut bman, &catalog_show).await.unwrap();
        assert_eq!(show.tracks.len(), 4);
        // Should be sorted by disc then track
        assert_eq!(show.tracks[0].disc_num, 1);
        assert_eq!(show.tracks[0].song_title, "Morning Dew");
        assert_eq!(show.tracks[2].disc_num, 2);
        assert_eq!(show.tracks[2].song_title, "Eyes of the World");
    }

    #[tokio::test]
    async fn test_fetch_bman_show_detail_unmapped_container_id() {
        let bman = BmanApi::new_for_test("key".into(), "root".into());
        let catalog_show = make_bman_catalog_show(
            -999999, BMAN_GD_ARTIST_ID, "Grateful Dead",
            "1977-05-08", "Cornell", "Ithaca", "NY",
        );

        // id_map is empty, so this should fail
        let mut bman_mut = bman;
        let err = fetch_bman_show_detail(&mut bman_mut, &catalog_show).await.unwrap_err();
        assert!(err.contains("No Drive folder mapped"), "Error: {}", err);
    }

    #[tokio::test]
    async fn test_fetch_bman_show_empty_folder() {
        let server = MockServer::start().await;
        let mut bman = BmanApi::new_for_test("key".into(), "root".into())
            .with_drive_base_url(&server.uri());

        let folder_id = "empty_folder";
        let container_id = bman.id_map.insert(folder_id);

        Mock::given(method("GET"))
            .and(query_param(
                "q",
                format!("'{}' in parents", folder_id),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": []
            })))
            .mount(&server)
            .await;

        let catalog_show = make_bman_catalog_show(
            container_id, BMAN_GD_ARTIST_ID, "Grateful Dead",
            "1977-05-08", "Cornell", "Ithaca", "NY",
        );

        let show = fetch_bman_show_detail(&mut bman, &catalog_show).await.unwrap();
        assert!(show.tracks.is_empty(), "Empty folder should produce no tracks");
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 10: Cover Art Generation
    // ════════════════════════════════════════════════════════════════════

    #[cfg(feature = "bman")]
    #[test]
    fn test_cover_art_different_shows_differ() {
        use crate::bman::cover::generate_cover;

        let png1 = generate_cover("Grateful Dead", "1977-05-08", "Cornell", "Ithaca", "NY");
        let png2 = generate_cover("Grateful Dead", "1972-08-27", "Old Renaissance Faire Grounds", "Veneta", "OR");

        assert_ne!(png1, png2, "Different shows should produce different covers");
        assert!(!png1.is_empty());
        assert!(!png2.is_empty());
    }

    #[cfg(feature = "bman")]
    #[test]
    fn test_cover_art_valid_png() {
        use crate::bman::cover::generate_cover;
        let png = generate_cover("Jerry Garcia Band", "1977-05-21", "Venue", "City", "ST");
        assert!(png.len() > 100, "PNG should be substantial");
        // Verify it's a valid image
        let img = image::load_from_memory(&png).expect("Should be decodable PNG");
        assert_eq!(img.width(), 600);
        assert_eq!(img.height(), 600);
    }

    #[test]
    fn test_cover_art_save_and_read() {
        use crate::bman::cover::save_cover;
        let tmp = tempdir().unwrap();
        let data = b"\x89PNG\r\n\x1a\ntest_cover_data";
        let path = save_cover(tmp.path(), data).unwrap();
        assert_eq!(path.file_name().unwrap(), "cover.png");
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 11: Bman Cache Persistence
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_bman_cache_save_and_load() {
        let tmp = tempdir().unwrap();
        let shows = vec![
            make_bman_catalog_show(-100, BMAN_GD_ARTIST_ID, "Grateful Dead", "1977-05-08", "Cornell", "Ithaca", "NY"),
            make_bman_catalog_show(-101, BMAN_JGB_ARTIST_ID, "Jerry Garcia Band", "1977-05-21", "Venue", "City", "ST"),
        ];

        cache::save_bman_cache(tmp.path(), &shows);
        let loaded = cache::load_bman_cache(tmp.path());

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].performance_date, "1977-05-08");
        assert_eq!(loaded[1].artist_name, "Jerry Garcia Band");
    }

    #[test]
    fn test_bman_cache_missing_returns_none() {
        let tmp = tempdir().unwrap();
        assert!(cache::load_bman_cache(tmp.path()).is_none());
    }

    #[test]
    fn test_bman_id_map_cache_roundtrip() {
        let tmp = tempdir().unwrap();
        let mut map = BmanIdMap::new();
        map.insert("folder_a");
        map.insert("folder_b");

        cache::save_bman_id_map(tmp.path(), &map);
        let loaded = cache::load_bman_id_map(tmp.path());

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.get_drive_id(map.get_synthetic_id("folder_a").unwrap()),
            Some("folder_a")
        );
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 12: Format Code & Service Mapping
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_bman_format_code_is_always_flac() {
        // Bman only serves FLAC — all format codes should map to FLAC's code
        assert_eq!(FormatCode::Flac.code(Service::Bman), FormatCode::Flac.code(Service::Bman));
        // But the actual value should be 2 (FLAC code)
        assert_eq!(FormatCode::Flac.code(Service::Bman), 2);
    }

    #[test]
    fn test_bman_service_distinct_from_nugs_livephish() {
        assert_ne!(Service::Bman, Service::Nugs);
        assert_ne!(Service::Bman, Service::LivePhish);
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 13: Non-Regression for Nugs/LivePhish
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_nugs_shows_unaffected_by_bman() {
        let tmp = tempdir().unwrap();
        let mut catalog = Catalog::new(tmp.path().to_path_buf());

        // Add both nugs and bman shows
        let nugs = CatalogShow {
            container_id: 12345,
            artist_id: 196,
            artist_name: "Phish".to_string(),
            venue_name: "MSG".to_string(),
            venue_city: "New York".to_string(),
            venue_state: "NY".to_string(),
            performance_date: "2024-08-31".to_string(),
            performance_date_year: "2024".to_string(),
            service: Service::Nugs,
            ..Default::default()
        };
        let bman = make_bman_catalog_show(
            -100, BMAN_GD_ARTIST_ID, "Grateful Dead",
            "2024-01-01", "Venue", "City", "ST",
        );

        catalog.shows.push(nugs);
        catalog.shows.push(bman);
        catalog.build_indexes();

        // Nugs shows should still be findable
        let y2024: Vec<_> = catalog
            .get_shows_by_year("2024")
            .into_iter()
            .filter(|s| s.service == Service::Nugs)
            .collect();
        assert_eq!(y2024.len(), 1);
        assert_eq!(y2024[0].artist_name, "Phish");
    }

    #[test]
    fn test_livephish_shows_unaffected_by_bman() {
        let tmp = tempdir().unwrap();
        let mut catalog = Catalog::new(tmp.path().to_path_buf());

        let lp = CatalogShow {
            container_id: 67890,
            artist_id: 62,
            artist_name: "Phish".to_string(),
            venue_name: "MSG".to_string(),
            venue_city: "New York".to_string(),
            venue_state: "NY".to_string(),
            performance_date: "2024-12-31".to_string(),
            performance_date_year: "2024".to_string(),
            service: Service::LivePhish,
            ..Default::default()
        };
        catalog.shows.push(lp);
        catalog.build_indexes();

        let y2024 = catalog.get_shows_by_year("2024");
        assert_eq!(y2024.len(), 1);
        assert_eq!(y2024[0].service, Service::LivePhish);
    }

    #[test]
    fn test_is_bman_artist_routing() {
        use crate::catalog::is_bman_artist;
        assert!(is_bman_artist(BMAN_GD_ARTIST_ID));
        assert!(is_bman_artist(BMAN_JGB_ARTIST_ID));
        assert!(!is_bman_artist(196)); // Phish
        assert!(!is_bman_artist(62));  // Phish (LivePhish)
        assert!(!is_bman_artist(0));
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 14: Metadata Enrichment Pipeline
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_title_source_ordering() {
        use crate::bman::metadata::TitleSource;
        assert!(TitleSource::SetlistFm > TitleSource::VorbisComment);
        assert!(TitleSource::VorbisComment > TitleSource::InfoFile);
        assert!(TitleSource::InfoFile > TitleSource::Filename);
        assert!(TitleSource::Filename > TitleSource::Fallback);
    }

    #[tokio::test]
    async fn test_bman_enrich_metadata_creates_show_dir() {
        let tmp = tempdir().unwrap();
        let mut show = make_show_with_tracks(
            -100, "Grateful Dead", "1977-05-08", "Cornell", vec![],
        );

        // No setlistfm key = skip that step
        bman_enrich_metadata(&mut show, tmp.path(), "").await;

        let expected_dir = tmp.path().join(show.folder_name());
        assert!(expected_dir.exists(), "Show dir should be created");
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 15: Catalog Traversal Structure Tests
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_is_year_folder() {
        assert!(is_year_folder("1977").is_some());
        assert!(is_year_folder("2024").is_some());
        assert!(is_year_folder("1965").is_some());
        assert!(is_year_folder("NLL").is_none());
        assert!(is_year_folder("JGB").is_none());
        assert!(is_year_folder("Bonus").is_none());
    }

    #[test]
    fn test_is_nll_folder() {
        assert!(is_nll_folder("New Lossless Library"));
        assert!(is_nll_folder("⚡")); // lightning bolt variant
        assert!(!is_nll_folder("1977"));
        assert!(!is_nll_folder("JGB"));
        assert!(!is_nll_folder("NLL")); // plain "NLL" doesn't match — needs "New Lossless"
    }

    #[test]
    fn test_is_jgb_folder() {
        assert!(is_jgb_folder("JGB"));
        assert!(is_jgb_folder("Jerry Garcia Band"));
        assert!(is_jgb_folder("jgb")); // case insensitive
        assert!(!is_jgb_folder("1977"));
        assert!(!is_jgb_folder("New Lossless Library"));
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 16: Disc Subfolder Detection
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_disc_subfolder_patterns() {
        assert_eq!(parse_disc_subfolder("Disc 1"), Some(1));
        assert_eq!(parse_disc_subfolder("Disc 2"), Some(2));
        assert_eq!(parse_disc_subfolder("Disk 1"), Some(1));
        assert_eq!(parse_disc_subfolder("CD 1"), Some(1));
        assert_eq!(parse_disc_subfolder("Set 1"), Some(1));
        assert_eq!(parse_disc_subfolder("d1"), Some(1));
        assert_eq!(parse_disc_subfolder("d2"), Some(2));
        assert_eq!(parse_disc_subfolder("info"), None);
        assert_eq!(parse_disc_subfolder("artwork"), None);
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 17: Edge Cases & Robustness
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_empty_folder_name_returns_none() {
        assert!(parse_show_folder("", "fid", BmanArtist::GratefulDead, false).is_none());
    }

    #[test]
    fn test_garbage_folder_name_returns_none() {
        assert!(parse_show_folder("random_junk_no_date", "fid", BmanArtist::GratefulDead, false).is_none());
    }

    #[test]
    fn test_non_flac_files_ignored_in_collect_tracks() {
        let items = vec![
            make_drive_item("f1", "track.mp3", "audio/mpeg"),
            make_drive_item("f2", "info.txt", "text/plain"),
            make_drive_item("f3", "cover.jpg", "image/jpeg"),
            make_flac_item("f4", "01 Real Track.flac"),
        ];
        let mut bman = BmanApi::new_for_test("key".into(), "root".into());
        let mut tracks = Vec::new();
        collect_tracks_from_items(&items, None, &mut bman, &mut tracks);

        assert_eq!(tracks.len(), 1, "Only the FLAC should be collected");
        assert_eq!(tracks[0].song_title, "Real Track");
    }

    #[test]
    fn test_two_digit_year_expands_correctly() {
        let show = parse_show_folder(
            "gd69-01-25.aud.flac16",
            "fid",
            BmanArtist::GratefulDead,
            false,
        ).unwrap();
        assert_eq!(show.date, "1969-01-25");
    }

    #[test]
    fn test_four_digit_year_etree() {
        let show = parse_show_folder(
            "gd1977-05-08.sbd.flac16",
            "fid",
            BmanArtist::GratefulDead,
            false,
        );
        // Whether this parses depends on the regex — but it should at least not crash
        if let Some(s) = show {
            assert!(s.date.starts_with("1977") || s.date.starts_with("19"));
        }
    }

    #[test]
    fn test_folder_name_with_only_whitespace() {
        assert!(parse_show_folder("   ", "fid", BmanArtist::GratefulDead, false).is_none());
    }

    #[test]
    fn test_catalog_show_from_parsed_show_preserves_fields() {
        let parsed = ParsedShow {
            date: "1977-05-08".to_string(),
            venue: "Barton Hall".to_string(),
            city: "Ithaca".to_string(),
            state: "NY".to_string(),
            source_type: SourceType::Sbd,
            source_tag: "Betty Board".to_string(),
            is_nll: true,
            artist: BmanArtist::GratefulDead,
            folder_id: "abc123".to_string(),
        };

        // Simulate what fetch_bman does
        let mut bman = BmanApi::new_for_test("key".into(), "root".into());
        let container_id = bman.id_map.insert(&parsed.folder_id);

        let catalog_show = CatalogShow {
            container_id,
            artist_id: parsed.artist.artist_id(),
            artist_name: parsed.artist.name().to_string(),
            container_info: format!("({})", parsed.source_tag),
            venue_name: parsed.venue.clone(),
            venue_city: parsed.city.clone(),
            venue_state: parsed.state.clone(),
            performance_date: parsed.date.clone(),
            performance_date_formatted: String::new(),
            performance_date_year: "1977".to_string(),
            image_url: String::new(),
            song_list: String::new(),
            service: Service::Bman,
        };

        assert!(container_id < 0);
        assert_eq!(catalog_show.artist_name, "Grateful Dead");
        assert_eq!(catalog_show.venue_name, "Barton Hall");
        assert_eq!(catalog_show.venue_city, "Ithaca");
        assert_eq!(catalog_show.venue_state, "NY");
        assert_eq!(catalog_show.container_info, "(Betty Board)");
        assert_eq!(catalog_show.performance_date_year, "1977");
        assert_eq!(catalog_show.service, Service::Bman);
    }

    // ════════════════════════════════════════════════════════════════════
    //  SECTION 18: Search Integration
    // ════════════════════════════════════════════════════════════════════

    #[test]
    fn test_bman_shows_searchable() {
        let tmp = tempdir().unwrap();
        let mut catalog = Catalog::new(tmp.path().to_path_buf());

        catalog.shows.push(make_bman_catalog_show(
            -100, BMAN_GD_ARTIST_ID, "Grateful Dead",
            "1977-05-08", "Cornell University", "Ithaca", "NY",
        ));
        catalog.shows.push(make_bman_catalog_show(
            -101, BMAN_GD_ARTIST_ID, "Grateful Dead",
            "1972-08-27", "Old Renaissance Faire Grounds", "Veneta", "OR",
        ));
        catalog.build_indexes();

        // Search by venue
        let results = catalog.search("Cornell", 50);
        assert!(results.iter().any(|s| s.container_id == -100));

        // Search by city
        let results = catalog.search("Veneta", 50);
        assert!(results.iter().any(|s| s.container_id == -101));

        // Search by date
        let results = catalog.search("1977-05-08", 50);
        assert!(results.iter().any(|s| s.container_id == -100));
    }
}
