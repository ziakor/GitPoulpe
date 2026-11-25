use gitpoulpe_core::GraphRow;
use rune::{Color, El, LineCap, Path, Shape, canvas};

use crate::theme::Palette;

pub const LANE_WIDTH: f32 = 16.0;
const STROKE: f32 = 2.0;
const DOT: f32 = 4.5;

fn x(lane: usize) -> f32 {
    lane as f32 * LANE_WIDTH + LANE_WIDTH / 2.0
}

fn bend(shape: Shape, from: [f32; 2], to: [f32; 2], color: Color) -> Shape {
    let half = (to[1] - from[1]) / 2.0;
    let path =
        Path::new()
            .move_to(from)
            .cubic_to([from[0], from[1] + half], [to[0], to[1] - half], to);
    shape.stroke_path(&path, STROKE, LineCap::Round, color)
}

fn disc(center: [f32; 2], radius: f32) -> Vec<[f32; 2]> {
    (0..20)
        .map(|i| {
            let angle = i as f32 / 20.0 * std::f32::consts::TAU;
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        })
        .collect()
}

pub fn graph_cell(p: &Palette, row: &GraphRow, lanes: usize, height: f32, merge: bool) -> El {
    let middle = height / 2.0;
    let mut shape = Shape::new();
    for lane in &row.through {
        shape = shape.line(
            [x(*lane), 0.0],
            [x(*lane), height],
            STROKE,
            LineCap::Butt,
            p.lane(*lane),
        );
    }
    let own = p.lane(row.column);
    let column = x(row.column);
    if row.entered {
        shape = shape.line([column, 0.0], [column, middle], STROKE, LineCap::Butt, own);
    }
    if row.continues {
        shape = shape.line(
            [column, middle],
            [column, height],
            STROKE,
            LineCap::Butt,
            own,
        );
    }
    for lane in &row.closing {
        shape = bend(shape, [x(*lane), 0.0], [column, middle], p.lane(*lane));
    }
    for lane in row.opening.iter().chain(&row.linking) {
        shape = bend(shape, [column, middle], [x(*lane), height], p.lane(*lane));
    }
    shape = if merge {
        shape.stroke_with(
            disc([column, middle], DOT - STROKE / 2.0),
            true,
            STROKE,
            LineCap::Butt,
            own,
        )
    } else {
        shape.fill(disc([column, middle], DOT), own)
    };
    canvas(shape).w(lanes.max(1) as f32 * LANE_WIDTH).h(height)
}
