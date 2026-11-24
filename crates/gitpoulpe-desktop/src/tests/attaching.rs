use rune::Runtime;

use super::requesting::{Fake, click, loaded, repo, screen, settle, shows, text_of};
use super::{click_at, rect_of, step};
use crate::attachments::Media;
use crate::state::State;

const U1: &str = "fd0fc972-c7df-42cc-b025-35deb408c0b6";
const U2: &str = "0a0fbb9c-3fe6-443d-9621-749ebd906115";

fn settled(s: &State) -> bool {
    let media = &s.pulls.list.media;
    media.len() == 6
        && media
            .values()
            .all(|m| matches!(m, Media::Ready(..) | Media::Failed(_)))
}

fn press(rt: &mut Runtime<State>, key: &str) {
    let target = key.to_owned();
    rt.update(move |_, tx| tx.scroll_into_view_in("prl-panel", target));
    step(rt);
    let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not on screen"));
    click_at(rt, rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0, false);
}

#[test]
fn attachments_show_in_place_and_open_in_the_browser_and_late_images_are_dropped() {
    let fake = Fake::start_script("fake_github.py");
    let dir = repo(&format!("{}/acme/proj.git", fake.base()), "feature/x");
    let mut rt = screen(dir.path(), &fake, &["Octo"]);
    click(&mut rt, "pr-list");
    settle(&mut rt, |s| s.pulls.list.open && !s.pulls.list.loading);
    rt.state.select_pull(7);
    loaded(&mut rt, 7);
    settle(&mut rt, settled);

    assert!(shows(&rt, "Avant :") && shows(&rt, "Fin."));
    assert!(
        !shows(&rt, "user-attachments"),
        "no raw attachment syntax is left"
    );
    let drawn = |rt: &Runtime<State>, keys: &[&str]| {
        for key in keys {
            let rect = rect_of(rt, key).unwrap_or_else(|| panic!("{key} is not drawn"));
            assert!(rect[2] > 0.0 && rect[3] > 0.0, "{key} has no size");
        }
    };
    drawn(
        &rt,
        &[
            "prl-body-a1-image",
            "prl-body-a7-image",
            "prc-general0-a1-image",
        ],
    );
    assert!(shows(&rt, "Capture"));
    assert!(shows(&rt, "Vidéo « demo.mp4 »"));
    assert!(shows(&rt, "Fichier joint « trace.zip »"));
    assert!(shows(
        &rt,
        "Image « gone » non affichée : ressource introuvable"
    ));
    assert_eq!(text_of(&rt, "prc-general0-t0").as_deref(), Some("Écran"));
    rt.take_url_requests();
    press(&mut rt, "prl-body-a3-open");
    press(&mut rt, "prl-body-a1-image");

    click(&mut rt, "prl-tab-files");
    step(&mut rt);
    drawn(
        &rt,
        &[
            "prc-f0c71-a1-image",
            "prl-file3-before-image",
            "prl-file3-after-image",
        ],
    );
    assert_eq!(text_of(&rt, "prc-f0c71-t0").as_deref(), Some("zoom"));
    assert!(shows(&rt, "Avant") && shows(&rt, "Après"));
    let sizes: Vec<[u32; 2]> = ["before", "after"]
        .iter()
        .map(|side| {
            let rect = rect_of(&rt, &format!("prl-file3-{side}-image")).unwrap();
            [rect[2].round() as u32, rect[3].round() as u32]
        })
        .collect();
    assert_eq!(sizes, [[6, 3], [8, 4]], "each side shows its own revision");
    press(&mut rt, "prl-file4-bin-open");
    assert_eq!(
        rt.take_url_requests(),
        [
            format!("{}/user-attachments/assets/{U2}", fake.base()),
            format!("{}/user-attachments/assets/{U1}", fake.base()),
            format!("{}/acme/proj/blob/sha7/assets/demo.mp4", fake.base()),
        ]
    );

    let late = rt
        .state
        .images
        .as_ref()
        .unwrap()
        .rgba(2, 3, vec![200u8; 24]);
    late.wait().unwrap();
    rt.state
        .pulls
        .list
        .media
        .insert("late".to_owned(), Media::Decoding(late));
    assert!(
        rt.state.drain(),
        "a decode finishing between events redraws the view"
    );
    assert!(matches!(
        rt.state.pulls.list.media.get("late"),
        Some(Media::Ready(_, [2, 3]))
    ));

    rt.state.select_pull(1);
    assert!(rt.state.pulls.list.media.is_empty());
    let link = format!("{}/user-attachments/assets/{U1}", fake.base());
    assert!(!rt.state.apply_pull_media(7, link, Ok(Vec::new())));
}
