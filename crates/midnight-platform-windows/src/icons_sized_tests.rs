//! Shell artwork at the edges rows, tiles and choices draw it at.
use super::{tests::native_qualification, *};
use midnight_platform_common::icon_scale;

/// Edges drawn at 100 % to 225 %: rows of 20 and 24 px, location buttons, tiles of
/// 64 to 96 px, and the largest tile.
const EDGES: [u16; 8] = [16, 20, 32, 45, 96, 144, 216, 256];

fn keys() -> Vec<IconKey> {
    let current = std::env::current_dir().unwrap();
    let root = current.ancestors().last().unwrap().to_owned();
    vec![
        IconKey {
            extension: Some("txt".into()),
            ..IconKey::file()
        },
        IconKey::file(),
        IconKey::folder(),
        IconKey::folder_link(),
        IconKey::for_drive(&root),
        IconKey {
            kind: IconKind::Location(LocationKind::Home),
            ..IconKey::folder()
        },
        IconKey::action(ActionIcon::Delete),
    ]
}

#[test]
fn shell_artwork_is_rendered_at_every_edge_it_is_drawn_at() {
    let _qualification = native_qualification();
    let services = WindowsFileSystem::new(None);
    for key in keys() {
        for edge in EDGES {
            let pixels = services
                .load_icon(&key.clone().sized(edge))
                .unwrap()
                .unwrap_or_else(|| panic!("{key:?} at {edge} px"));
            assert_eq!(
                (pixels.width, pixels.height, pixels.rgba.len()),
                (
                    u32::from(edge),
                    u32::from(edge),
                    usize::from(edge).pow(2) * 4
                ),
                "{key:?}"
            );
            assert!(
                pixels
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] != 0),
                "{key:?} at {edge} px is transparent"
            );
        }
        assert_eq!(
            services.icon_edge(216),
            216,
            "drawn edges are rendered as they are"
        );
    }
    for edge in [0, MAX_ICON_EDGE + 1] {
        assert!(
            services
                .load_icon(&IconKey::folder().sized(edge))
                .unwrap()
                .is_none()
        );
    }
}

/// Neighboring pixels, across and down, that differ by at least a quarter of the
/// range in a premultiplied channel: the crisp outlines and details of artwork drawn
/// for the size. Enlarging a small bitmap spreads each such step over several pixels.
fn sharp_steps(rgba: &[u8], edge: usize) -> usize {
    let pixel = |x: usize, y: usize| {
        let [red, green, blue, alpha] = rgba.as_chunks::<4>().0[y * edge + x];
        let premultiply = |channel: u8| u32::from(channel) * u32::from(alpha) / 255;
        [
            premultiply(red),
            premultiply(green),
            premultiply(blue),
            u32::from(alpha),
        ]
    };
    let differ = |a: [u32; 4], b: [u32; 4]| a.iter().zip(b).any(|(a, b)| a.abs_diff(b) >= 64);
    let mut steps = 0;
    for y in 0..edge {
        for x in 0..edge {
            steps += usize::from(x + 1 < edge && differ(pixel(x, y), pixel(x + 1, y)));
            steps += usize::from(y + 1 < edge && differ(pixel(x, y), pixel(x, y + 1)));
        }
    }
    steps
}

#[test]
fn large_icons_carry_detail_of_their_own_not_an_enlarged_small_bitmap() {
    let _qualification = native_qualification();
    let services = WindowsFileSystem::new(None);
    for key in [
        IconKey::folder(),
        IconKey {
            extension: Some("txt".into()),
            ..IconKey::file()
        },
    ] {
        let large = services
            .load_icon(&key.clone().sized(216))
            .unwrap()
            .unwrap();
        let small = services.load_icon(&key.clone().sized(32)).unwrap().unwrap();
        // The bitmap a 32 px source gives a 216 px tile: what tiles drew before.
        let enlarged = icon_scale::fit(&small.rgba, 32, 32, 216).unwrap();
        let (own, stretched) = (sharp_steps(&large.rgba, 216), sharp_steps(&enlarged, 216));
        assert!(
            own > 2 * stretched,
            "{key:?}: {own} sharp steps at 216 px, {stretched} in the enlarged 32 px bitmap"
        );
    }
}

#[test]
fn artwork_without_a_jumbo_image_is_drawn_from_the_largest_true_bitmap() {
    let _qualification = native_qualification();
    // An icon file with one 32 px frame. The Shell keeps it in the jumbo list as the
    // extra-large bitmap in the top-left corner of an otherwise empty 256 px canvas.
    let directory = crate::test_fixtures::build_temp("small-icon-");
    let path = directory.path().join("small only.ico");
    let edge = 32u32;
    let mask = edge.div_ceil(32) * 4 * edge;
    let mut file = vec![0, 0, 1, 0, 1, 0, 32, 32, 0, 0, 1, 0, 32, 0];
    let size = 40 + edge * edge * 4 + mask;
    file.extend(size.to_le_bytes());
    file.extend(22u32.to_le_bytes());
    file.extend(40u32.to_le_bytes());
    file.extend((edge as i32).to_le_bytes());
    file.extend((edge as i32 * 2).to_le_bytes());
    file.extend([1, 0, 32, 0]);
    file.extend([0; 24]);
    for _ in 0..edge * edge {
        file.extend([40, 160, 230, 255]);
    }
    file.extend(vec![0; mask as usize]);
    std::fs::write(&path, file).unwrap();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let _com = ComGuard::initialize(COINIT_MULTITHREADED as u32).unwrap();
    // The icon file itself, as the Shell lists it: only this test passes a real path.
    let image = shell_image(&wide, 0, 0).expect("system image of the icon file");
    let rgba = lists::render(image, 216).expect("rendered");
    let visible = |x: usize, y: usize| rgba.as_chunks::<4>().0[y * 216 + x][3] != 0;
    assert!(
        visible(200, 200) && visible(108, 108) && visible(5, 5),
        "the artwork fills the tile instead of a corner of it"
    );
}

#[test]
fn unassociated_types_are_generic_at_every_edge_and_associated_ones_are_not() {
    let _qualification = native_qualification();
    let services = WindowsFileSystem::new(None);
    for edge in [20, 216] {
        let unknown = IconKey {
            extension: Some("midnight-unassociated-icon-fixture-7cb49".into()),
            ..IconKey::file().sized(edge)
        };
        if let Some(pixels) = services.load_icon(&unknown).unwrap() {
            assert!(pixels.generic, "{edge} px");
        }
        let folder = services
            .load_icon(&IconKey::folder().sized(edge))
            .unwrap()
            .unwrap();
        assert!(!folder.generic, "{edge} px folder");
    }
}
