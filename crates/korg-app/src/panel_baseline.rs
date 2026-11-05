//! Historical panel definitions, only compiled for explicit parity checks and export.
use super::{Binding, Control, ControlKind, Panel};

/// Original implementation retained for reproducible export and parity checks.
pub fn baseline_panel(class: &str) -> Option<Panel> {
    match class {
        "X10820" => Some(nanokey2()),
        "X10830" => Some(nanopad2()),
        "X10840" => Some(nanokontrol2()),
        "X11810" => Some(microkey25("X11810", 598.0, 211.0)),
        "X14800" => Some(nanokey_studio()),
        "X14810" => Some(microkey37_air()),
        "X14802" => Some(nanokontrol_studio()),
        "X14823" => Some(microkey25_air()),
        "X11850" => Some(taktile("X11850")),
        "X12880" => Some(taktile("X12880")),
        "X20870" => Some(keystage()),
        "X20880" => Some(nanokey_fold()),
        "X09800" => Some(microkey37()),
        "X19210" => Some(sq64()),
        "X21212" => Some(nts1()),
        "X21210" => Some(nts3()),
        _ => None,
    }
}

struct Builder {
    controls: Vec<Control>,
}

impl Builder {
    fn new() -> Self {
        Self {
            controls: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        group: &'static str,
        label: impl Into<String>,
        kind: ControlKind,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        binding: Binding,
    ) {
        self.controls.push(Control {
            group: group.to_owned(),
            label: label.into(),
            kind,
            x,
            y,
            w,
            h,
            binding,
        });
    }

    fn button(&mut self, group: &'static str, label: &str, x: f64, y: f64, w: f64, h: f64) {
        self.add(group, label, ControlKind::Button, x, y, w, h, Binding::None);
    }

    fn knob(&mut self, group: &'static str, label: String, cx: f64, cy: f64, r: f64) {
        let kind = ControlKind::Knob;
        self.add(
            group,
            label,
            kind,
            cx - r,
            cy - r,
            2.0 * r,
            2.0 * r,
            Binding::None,
        );
    }

    /// A key row: `white` keys from `x0` to `x1`, and black keys at `black_x`.
    #[allow(clippy::too_many_arguments)]
    fn keyboard(
        &mut self,
        _white: usize,
        x0: f64,
        x1: f64,
        white_y: (f64, f64),
        _black_x: &[f64],
        _black_w: f64,
        black_y: (f64, f64),
        binding: Binding,
    ) {
        let y0 = white_y.0.min(black_y.0);
        let y1 = (white_y.0 + white_y.1).max(black_y.0 + black_y.1);
        self.add(
            "Keyboard",
            "Keyboard",
            ControlKind::Key { black: false },
            x0,
            y0,
            x1 - x0,
            y1 - y0,
            binding,
        );
    }

    fn finish(self, class: &'static str, width: f64, height: f64, inferred: bool) -> Panel {
        Panel {
            class: class.to_owned(),
            width,
            height,
            controls: self.controls,
            binding_inferred: inferred,
        }
    }
}

/// nanoKONTROL Studio, panel picture 550 x 329.
fn nanokontrol_studio() -> Panel {
    let mut b = Builder::new();
    b.button("Scene", "Scene", 25.0, 93.0, 22.0, 12.0);
    for (label, x) in [
        ("Cycle", 25.0),
        ("Set", 57.0),
        ("Marker ◀", 88.0),
        ("Marker ▶", 121.0),
    ] {
        b.button("Transport", label, x, 116.0, 22.0, 11.0);
    }
    for (label, x) in [
        ("Rewind", 25.0),
        ("Fast Fwd", 57.0),
        ("Track ◀", 88.0),
        ("Track ▶", 121.0),
    ] {
        b.button("Transport", label, x, 137.0, 22.0, 11.0);
    }
    for (label, x) in [
        ("Return", 25.0),
        ("Stop", 57.0),
        ("Play", 88.0),
        ("Rec", 121.0),
    ] {
        b.button("Transport", label, x, 157.0, 22.0, 22.0);
    }
    b.add(
        "Jog Wheel",
        "Jog Wheel",
        ControlKind::Jog,
        36.0,
        199.0,
        92.0,
        92.0,
        Binding::Jog,
    );
    for channel in 0..8 {
        let cx = 180.0 + channel as f64 * 44.5;
        let n = channel + 1;
        for (group, name, y) in [
            ("Track Mute", "Mute", 54.0),
            ("Track Solo", "Solo", 75.0),
            ("Track Rec", "Rec", 95.0),
            ("Track Select", "Select", 116.0),
        ] {
            b.add(
                group,
                format!("{name} {n}"),
                ControlKind::Button,
                cx - 12.0,
                y,
                24.0,
                11.0,
                Binding::None,
            );
        }
        b.knob("Knob", format!("Knob {n}"), cx, 160.0, 14.0);
        b.add(
            "Slider",
            format!("Slider {n}"),
            ControlKind::Fader,
            cx - 7.0,
            188.0,
            14.0,
            116.0,
            Binding::None,
        );
    }
    b.finish("X14802", 550.0, 329.0, false)
}

/// nanoKEY Studio, panel picture 500 x 294.
fn nanokey_studio() -> Panel {
    let mut b = Builder::new();
    for row in 0..2 {
        for col in 0..4 {
            let n = row * 4 + col + 1;
            let cx = [38.0, 77.0, 116.0, 156.0][col];
            b.knob(
                "Knob",
                format!("Knob {n}"),
                cx,
                55.0 + row as f64 * 45.0,
                14.0,
            );
        }
    }
    // The original edits only these physical buttons; the rest of the button
    // row is inert, so the schematic leaves it out.
    for (group, label, x, w) in [
        ("Sustain", "Sustain", 68.0, 21.0),
        ("Touch Scale", "Touch Scale", 105.0, 20.0),
        ("Pitch/Mod", "Pitch/Mod", 150.0, 20.0),
    ] {
        b.button(group, label, x, 129.0, w, 11.0);
    }
    b.add(
        "X-Y Pad",
        "X-Y Pad",
        ControlKind::TouchPad,
        190.0,
        42.0,
        108.0,
        80.0,
        Binding::None,
    );
    for row in 0..2 {
        for col in 0..4 {
            let n = row * 4 + col + 1;
            b.add(
                "Trigger Pad",
                format!("Pad {n}"),
                ControlKind::Pad,
                [318.0, 358.0, 398.0, 438.0][col],
                [35.0, 77.0][row],
                35.0,
                37.0,
                Binding::None,
            );
        }
    }
    b.button("Chord Pad", "Chord Pad", 432.0, 128.0, 15.0, 9.0);
    b.keyboard(
        15,
        20.0,
        480.0,
        (210.0, 55.0),
        &[
            40.0, 69.0, 128.0, 158.0, 188.0, 247.0, 277.0, 336.0, 366.0, 396.0,
        ],
        27.0,
        (150.0, 52.0),
        Binding::None,
    );
    b.finish("X14800", 500.0, 294.0, false)
}

/// nanoKEY2, panel picture 705 x 185. The schema has one element, the
/// scene-wide settings, so every control shows it.
fn nanokey2() -> Panel {
    let mut b = Builder::new();
    // The six buttons of the picture; all edit the one scene element.
    for (group, label, x, y) in [
        ("Octave", "Octave Down", 26.0, 94.0),
        ("Octave", "Octave Up", 59.0, 94.0),
        ("Sustain Button", "Sustain Button", 92.0, 94.0),
        ("Pitch Button", "Pitch Button", 26.0, 130.0),
        ("Pitch Button", "Pitch Up", 59.0, 130.0),
        ("Mod Button", "Mod Button", 92.0, 130.0),
    ] {
        b.add(
            group,
            label,
            ControlKind::Button,
            x,
            y,
            25.0,
            25.0,
            Binding::Element(0),
        );
    }
    b.keyboard(
        15,
        133.0,
        676.0,
        (92.0, 68.0),
        &[
            150.0, 187.0, 260.0, 297.0, 334.0, 407.0, 444.0, 517.0, 554.0, 590.0,
        ],
        33.0,
        (15.0, 68.0),
        Binding::Element(0),
    );
    b.finish("X10820", 705.0, 185.0, false)
}

/// nanoPAD2, panel picture 705 x 190. Schema elements 0..15 are bound to
/// the pads in reading order; the factory notes prove the order.
fn nanopad2() -> Panel {
    let mut b = Builder::new();
    b.add(
        "X-Y Pad",
        "X-Y Pad",
        ControlKind::TouchPad,
        27.0,
        45.0,
        140.0,
        108.0,
        Binding::None,
    );
    let xs = [195.0, 255.0, 316.0, 377.0, 443.0, 504.0, 565.0, 625.0];
    for (row, y) in [47.0, 107.0].into_iter().enumerate() {
        for (col, &x) in xs.iter().enumerate() {
            let index = row * 8 + col;
            b.add(
                "Trigger Pad",
                format!("Pad {}", index + 1),
                ControlKind::Pad,
                x,
                y,
                50.0,
                51.0,
                Binding::Element(index),
            );
        }
    }
    for (label, x) in [
        ("Hold", 395.0),
        ("Gate Arp", 427.0),
        ("Touch Scale", 460.0),
        ("Key/Range", 497.0),
        ("Scale/Tap", 531.0),
        ("Scene", 577.0),
    ] {
        b.button("Button", label, x, 15.0, 25.0, 11.0);
    }
    b.finish("X10830", 705.0, 190.0, false)
}

/// nanoKONTROL2, panel picture 780 x 210.
fn nanokontrol2() -> Panel {
    let mut b = Builder::new();
    for (label, x, y, w) in [
        ("Track ◀", 33.0, 86.0, 27.0),
        ("Track ▶", 69.0, 86.0, 27.0),
        ("Cycle", 27.0, 118.0, 33.0),
        ("Set", 106.0, 118.0, 26.0),
        ("Marker ◀", 143.0, 118.0, 27.0),
        ("Marker ▶", 180.0, 118.0, 27.0),
    ] {
        b.button("Transport", label, x, y, w, 11.0);
    }
    for (label, x) in [
        ("Rewind", 30.0),
        ("Fast Fwd", 68.0),
        ("Stop", 105.0),
        ("Play", 142.0),
        ("Rec", 180.0),
    ] {
        b.button("Transport", label, x, 146.0, 28.0, 26.0);
    }
    for channel in 0..8 {
        let cx = 270.0 + channel as f64 * 65.0;
        let n = channel + 1;
        b.knob("Knob", format!("Knob {n}"), cx, 37.0, 13.0);
        b.add(
            "Slider",
            format!("Slider {n}"),
            ControlKind::Fader,
            cx - 8.0,
            62.0,
            16.0,
            100.0,
            Binding::None,
        );
        for (group, name, y) in [
            ("Solo", "S", 80.0),
            ("Mute", "M", 114.0),
            ("Rec", "R", 148.0),
        ] {
            b.add(
                group,
                format!("{name} {n}"),
                ControlKind::Button,
                cx - 37.0,
                y,
                17.0,
                17.0,
                Binding::None,
            );
        }
    }
    b.finish("X10840", 780.0, 210.0, false)
}

/// microKEY-25, panel picture 598 x 211. A joystick, two assignable buttons,
/// the octave pair and 15 white keys. Also used for microKEY2-25 Air, which
/// has the same picture layout.
fn microkey25(class: &'static str, width: f64, height: f64) -> Panel {
    let mut b = Builder::new();
    let joystick = ControlKind::Jog;
    b.add(
        "Joystick",
        "Joystick",
        joystick,
        42.0,
        76.0,
        44.0,
        44.0,
        Binding::None,
    );
    for (group, label, x, y) in [
        ("Arpeggiator", "Arpeggiator", 35.0, 132.0),
        ("Sustain Button", "Sustain", 71.0, 132.0),
        ("Octave", "Octave Down", 35.0, 158.0),
        ("Octave", "Octave Up", 72.0, 158.0),
    ] {
        b.button(group, label, x, y, 11.0, 12.0);
    }
    b.keyboard(
        15,
        105.0,
        550.0,
        (68.0, 114.0),
        &[
            125.0, 161.0, 213.0, 247.0, 282.0, 333.0, 369.0, 421.0, 455.0, 490.0,
        ],
        13.0,
        (67.0, 61.0),
        Binding::None,
    );
    b.finish(class, width, height, false)
}

/// microKEY2-25 Air, panel picture 598 x 211. Same control layout as the
/// microKEY-25 picture.
fn microkey25_air() -> Panel {
    microkey25("X14823", 598.0, 211.0)
}

/// microKEY-37/61, panel picture 695 x 185. The picture shows the 37-key
/// model: pitch and mod wheels, an octave pair and 22 white keys.
fn microkey37() -> Panel {
    let mut b = Builder::new();
    let wheel = ControlKind::Jog;
    b.add(
        "Wheel",
        "Pitch",
        wheel,
        37.0,
        53.0,
        19.0,
        55.0,
        Binding::None,
    );
    b.add("Wheel", "Mod", wheel, 68.0, 53.0, 18.0, 55.0, Binding::None);
    b.button("Octave", "Octave Down", 38.0, 122.0, 9.0, 13.0);
    b.button("Octave", "Octave Up", 70.0, 122.0, 9.0, 13.0);
    b.keyboard(
        22,
        112.0,
        640.0,
        (52.0, 92.0),
        &[
            128.0, 157.0, 197.0, 226.0, 254.0, 294.0, 325.0, 365.0, 394.0, 422.0, 464.0, 493.0,
            533.0, 562.0, 590.0,
        ],
        12.0,
        (52.0, 55.0),
        Binding::None,
    );
    b.finish("X09800", 695.0, 185.0, false)
}

/// microKEY2/Air, panel picture 705 x 183. The picture shows the 37-key
/// model: pitch and mod wheels, an octave pair and 22 white keys.
fn microkey37_air() -> Panel {
    let mut b = Builder::new();
    let wheel = ControlKind::Jog;
    b.add(
        "Wheel",
        "Pitch",
        wheel,
        42.0,
        52.0,
        13.0,
        66.0,
        Binding::None,
    );
    b.add("Wheel", "Mod", wheel, 73.0, 52.0, 13.0, 66.0, Binding::None);
    b.button("Octave", "Octave Down", 42.0, 130.0, 9.0, 11.0);
    b.button("Octave", "Octave Up", 73.0, 130.0, 9.0, 11.0);
    b.keyboard(
        22,
        118.0,
        652.0,
        (57.0, 93.0),
        &[
            133.0, 163.0, 205.0, 231.0, 262.0, 304.0, 334.0, 374.0, 402.0, 433.0, 475.0, 505.0,
            547.0, 573.0, 604.0,
        ],
        12.0,
        (56.0, 49.0),
        Binding::None,
    );
    b.finish("X14810", 705.0, 183.0, false)
}

/// taktile-25/49 and TRITON taktile-25/49, panel picture 892 x 197. The
/// picture is the control surface above the keys: eight strips, a master
/// fader, a display, eight pads and the transport.
fn taktile(class: &'static str) -> Panel {
    let mut b = Builder::new();
    b.add(
        "Slider",
        "Master",
        ControlKind::Fader,
        5.0,
        48.0,
        23.0,
        104.0,
        Binding::None,
    );
    for (index, cx) in [57.0, 93.0, 129.0, 165.0, 201.0, 237.0, 273.0, 309.0]
        .iter()
        .enumerate()
    {
        let n = index + 1;
        b.knob("Knob", format!("Knob {n}"), *cx, 18.0, 18.0);
        b.add(
            "Slider",
            format!("Slider {n}"),
            ControlKind::Fader,
            cx - 5.0,
            62.0,
            10.0,
            70.0,
            Binding::None,
        );
    }
    for (label, y) in [
        ("Scene 1", 11.0),
        ("Scene 2", 43.0),
        ("Scene 3", 77.0),
        ("Scene 4", 110.0),
    ] {
        b.button("Scene", label, 334.0, y, 31.0, 19.0);
    }
    // The large square is the touch pad; the two keys at the ends of the
    // strip below it are the page buttons.
    b.add(
        "Touch Pad",
        "Touch Pad",
        ControlKind::TouchPad,
        381.0,
        25.0,
        120.0,
        92.0,
        Binding::None,
    );
    for (label, x) in [("Page −", 378.0), ("Page +", 492.0)] {
        b.button("Page", label, x, 158.0, 14.0, 10.0);
    }
    for row in 0..2 {
        for (column, x) in [510.0, 554.0, 598.0, 643.0, 700.0, 745.0, 790.0, 836.0]
            .iter()
            .enumerate()
        {
            let n = row * 8 + column + 1;
            let (y, h) = if row == 0 { (12.0, 45.0) } else { (70.0, 50.0) };
            b.add(
                "Trigger Pad",
                format!("Pad {n}"),
                ControlKind::Pad,
                *x,
                y,
                44.0,
                h,
                Binding::None,
            );
        }
    }
    for index in 0..8 {
        let n = index + 1;
        b.button(
            "Channel",
            &format!("Channel {n}"),
            44.0 + index as f64 * 35.4,
            165.0,
            31.0,
            20.0,
        );
    }
    for (label, x, w) in [
        ("Marker Set", 518.0, 32.0),
        ("Fast Fwd", 600.0, 25.0),
        ("Stop", 634.0, 28.0),
        ("Play", 672.0, 29.0),
        ("Rec", 709.0, 29.0),
        ("Loop", 746.0, 30.0),
    ] {
        b.button("Transport", label, x, 166.0, w, 20.0);
    }
    b.finish(class, 892.0, 197.0, false)
}

/// Keystage, panel picture 800 x 200. The picture crops the keyboard to 22
/// white keys; the wheels at the lower left are cut by its edge.
fn keystage() -> Panel {
    let mut b = Builder::new();
    b.knob("Knob", "Volume".to_string(), 31.0, 40.0, 7.0);
    for (label, x, w) in [
        ("Settings", 56.0, 28.0),
        ("Write", 87.0, 25.0),
        ("Exit", 113.0, 22.0),
        ("Shift", 142.0, 25.0),
        ("Undo", 115.0, 25.0),
    ] {
        let y = if label == "Undo" { 52.0 } else { 36.0 };
        b.button("Panel", label, x, y, w, 12.0);
    }
    for (label, x, w) in [
        ("Play", 65.0, 18.0),
        ("Stop", 87.0, 17.0),
        ("Rec", 113.0, 18.0),
        ("Fast Fwd", 137.0, 22.0),
        ("Tempo", 171.0, 25.0),
    ] {
        b.button("Transport", label, x, 82.0, w, 14.0);
    }
    b.button("Arp", "Arp", 215.0, 39.0, 29.0, 13.0);
    b.button("Arp", "Arp Edit", 216.0, 55.0, 24.0, 8.0);
    b.button("Value", "Value Up", 267.0, 55.0, 16.0, 9.0);
    b.button("Value", "Value Down", 267.0, 72.0, 16.0, 9.0);
    b.button("Value", "Track", 264.0, 99.0, 24.0, 8.0);
    b.button("Chord", "Chord", 215.0, 79.0, 29.0, 13.0);
    b.button("Chord", "Chord Edit", 217.0, 95.0, 24.0, 8.0);
    b.button("Metronome", "Metronome", 171.0, 96.0, 25.0, 8.0);
    b.button("Page", "Page −", 306.0, 84.0, 25.0, 12.0);
    b.button("Page", "Page +", 334.0, 84.0, 26.0, 12.0);
    b.button("Page", "Scene", 321.0, 98.0, 25.0, 8.0);
    b.knob("Knob", "Value".to_string(), 395.5, 45.5, 17.5);
    for index in 0..8 {
        let n = index + 1;
        b.knob(
            "Knob",
            format!("Knob {n}"),
            440.0 + index as f64 * 47.2,
            47.0,
            7.0,
        );
    }
    b.button("Octave", "Octave Down", 21.0, 144.0, 19.0, 8.0);
    b.button("Octave", "Octave Up", 67.0, 144.0, 19.0, 8.0);
    b.button("Octave", "Transpose", 42.0, 155.0, 24.0, 7.0);
    b.add(
        "Wheel",
        "Pitch",
        ControlKind::Jog,
        20.0,
        176.0,
        22.0,
        24.0,
        Binding::None,
    );
    b.add(
        "Wheel",
        "Mod",
        ControlKind::Jog,
        67.0,
        176.0,
        22.0,
        24.0,
        Binding::None,
    );
    b.keyboard(
        22,
        120.0,
        794.0,
        (124.0, 76.0),
        &[
            141.0, 179.0, 234.0, 269.0, 305.0, 360.0, 397.0, 452.0, 488.0, 524.0, 579.0, 616.0,
            671.0, 707.0, 743.0,
        ],
        13.0,
        (126.0, 62.0),
        Binding::None,
    );
    b.finish("X20870", 800.0, 200.0, false)
}

/// nanoKEY Fold, panel picture 700 x 183. Two touch strips, nine buttons,
/// ten knobs and fifteen pads; the labels are read from the picture.
fn nanokey_fold() -> Panel {
    let mut b = Builder::new();
    let strip = ControlKind::TouchPad;
    b.add(
        "Pitch/Mod",
        "Pitch",
        strip,
        50.0,
        20.0,
        160.0,
        18.0,
        Binding::None,
    );
    b.add(
        "Pitch/Mod",
        "Mod",
        strip,
        215.0,
        20.0,
        140.0,
        18.0,
        Binding::None,
    );
    // Nine round buttons on the top shelf, each with an LED dot above it;
    // the icons sit at their centre.
    for index in 0..9 {
        b.button(
            "Button",
            &format!("Button {}", index + 1),
            363.0 + index as f64 * 32.9,
            20.0,
            24.0,
            24.0,
        );
    }
    // The picture draws ten knob cards, in groups of two, three, two and
    // three. Each cap gets its own box; the cards do not align with the pads.
    for (label, x, w) in [
        ("Knob 1", 78.0, 35.0),
        ("Knob 2", 121.0, 34.0),
        ("C TYPE", 205.0, 35.0),
        ("C KEY", 248.0, 34.0),
        ("C STEP", 290.0, 35.0),
        ("A TYPE", 376.0, 34.0),
        ("A OCT", 418.0, 34.0),
        ("A PATTERN", 502.0, 36.0),
        ("A RATE", 545.0, 35.0),
        ("A GATE", 586.0, 37.0),
    ] {
        b.add(
            "Knob",
            label,
            ControlKind::Knob,
            x,
            62.0,
            w,
            54.0,
            Binding::None,
        );
    }
    for (index, x) in [
        55.0, 97.0, 140.0, 182.0, 225.0, 267.0, 309.0, 352.0, 395.0, 437.0, 480.0, 522.0, 564.0,
        606.0, 649.0,
    ]
    .iter()
    .enumerate()
    {
        b.add(
            "Touch Keyboard",
            format!("Pad {}", index + 1),
            ControlKind::Pad,
            *x,
            118.0,
            34.0,
            47.0,
            Binding::None,
        );
    }
    b.finish("X20880", 700.0, 183.0, false)
}

/// SQ-64, panel picture 3498 x 1225. Six knobs, the transport and mode
/// buttons, the four track buttons and the 64 step pads.
fn sq64() -> Panel {
    let mut b = Builder::new();
    b.button("Transport", "Tap", 102.0, 232.0, 153.0, 113.0);
    b.knob("Knob", "Tempo".to_string(), 393.0, 286.0, 63.0);
    b.knob("Knob", "Swing".to_string(), 689.0, 286.0, 63.0);
    for (label, x, w) in [
        ("Rec", 897.0, 125.0),
        ("Stop", 1053.0, 124.0),
        ("Play", 1209.0, 163.0),
    ] {
        b.button("Transport", label, x, 232.0, w, 113.0);
    }
    for (index, cx) in [2340.0, 2720.0, 3025.0, 3321.0].iter().enumerate() {
        b.knob("Knob", format!("Knob {}", index + 1), *cx, 286.0, 63.0);
    }
    for (label, x, w) in [
        ("Control", 100.0, 158.0),
        ("Gate", 315.0, 158.0),
        ("Pitch", 509.0, 159.0),
        ("Mod", 705.0, 158.0),
        ("Loop", 899.0, 160.0),
        ("1", 1484.0, 159.0),
        ("2", 1679.0, 159.0),
        ("3", 1874.0, 159.0),
        ("4", 2069.0, 159.0),
        ("Global", 2459.0, 159.0),
        ("Projects", 2653.0, 160.0),
        ("Patterns", 2848.0, 160.0),
        ("Shift", 3238.0, 159.0),
    ] {
        b.button("Mode", label, x, 452.0, w, 40.0);
    }
    let rows = [590.0, 730.0, 875.0, 1015.0];
    let columns = [
        334.0, 525.0, 723.0, 917.0, 1109.0, 1303.0, 1504.0, 1697.0, 1891.0, 2087.0, 2282.0, 2477.0,
        2675.0, 2866.0, 3060.0, 3255.0,
    ];
    for (index, letter) in ["A", "B", "C", "D"].iter().enumerate() {
        b.button(
            "Track",
            &format!("Track {letter}"),
            100.0,
            rows[index],
            150.0,
            100.0,
        );
        for (column, x) in columns.iter().enumerate() {
            b.add(
                "Step Pad",
                format!("{letter} {}", column + 1),
                ControlKind::Pad,
                *x,
                rows[index],
                128.0,
                100.0,
                Binding::None,
            );
        }
    }
    b.finish("X19210", 3498.0, 1225.0, false)
}

/// NTS-1 digital kit mkII, panel picture 130 x 82. Three large knobs, nine
/// parameter knobs and a ten-key keyboard.
fn nts1() -> Panel {
    let mut b = Builder::new();
    for (index, cx) in [65.0, 85.0, 105.0].iter().enumerate() {
        b.knob("Knob", format!("Knob {}", index + 1), *cx, 23.0, 7.0);
    }
    for (index, cx) in [20.5, 33.5, 46.0, 58.5, 71.5, 83.5, 96.5, 109.5]
        .iter()
        .enumerate()
    {
        b.knob(
            "Knob",
            format!("Parameter Knob {}", index + 1),
            *cx,
            45.0,
            6.0,
        );
    }
    b.keyboard(
        10,
        12.0,
        122.0,
        (63.0, 13.0),
        &[17.0, 42.0, 53.0, 73.0, 84.0, 95.0, 108.0],
        10.0,
        (53.0, 10.0),
        Binding::None,
    );
    b.finish("X21212", 130.0, 82.0, false)
}

/// NTS-3 kaoss pad kit, panel picture 96 x 90. The X-Y pad, five knobs,
/// the play button and the two side sliders.
fn nts3() -> Panel {
    let mut b = Builder::new();
    b.add(
        "X-Y Pad",
        "X-Y Pad",
        ControlKind::TouchPad,
        22.0,
        44.0,
        55.0,
        37.0,
        Binding::None,
    );
    // KORG's product photos show push buttons with LEDs for PERFORM, FX 1-4
    // and EDIT, a push encoder with a red cap, and pin headers (not
    // controls) on both side edges. The touch area holds a touch slider,
    // the X-Y pad and two touch buttons.
    b.button("Button", "Perform", 21.0, 13.0, 7.0, 7.0);
    for (index, cx) in [24.5, 37.0, 49.0, 61.5].iter().enumerate() {
        b.button(
            "Button",
            &format!("FX {}", index + 1),
            cx - 3.5,
            27.5,
            7.0,
            7.0,
        );
    }
    b.button("Button", "Edit", 70.5, 27.5, 7.0, 7.0);
    b.knob("Program", "Program".to_string(), 75.0, 17.0, 6.0);
    b.add(
        "Touch Slider",
        "Touch Slider",
        ControlKind::Fader,
        11.0,
        45.0,
        8.0,
        36.0,
        Binding::None,
    );
    b.button("Touch Button", "Touch Button 1", 79.0, 47.0, 8.0, 10.0);
    b.button("Touch Button", "Touch Button 2", 79.0, 58.0, 8.0, 10.0);
    b.finish("X21210", 96.0, 90.0, false)
}
