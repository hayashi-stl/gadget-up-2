use cgmath::prelude::*;
use cgmath::{vec2, vec4};
use fnv::{FnvHashMap, FnvHashSet};
use golem::Context;
use std::cell::{Cell, Ref, RefCell};
use std::rc::Rc;

use crate::camera::Camera;
use crate::grid::{Grid, WH, XY};
use crate::log;
use crate::math::{Mat4, Vec2, Vec2i, Vector2Ex};
use crate::model::Model;
use crate::shape::{Circle, Path, Rectangle, Shape};

pub type Port = u32;
pub type State = u32;

/// Type for (state, port) combinations
pub type SP = (Port, State);

/// Type for (port, port) traversals
pub type PP = (Port, Port);

/// Type for ((state, port), (state, port)) traversals
pub type SPSP = (SP, SP);

/// Definition of a gadget, including ports, states, and transitions
#[derive(Clone, Debug)]
pub struct GadgetDef {
    num_ports: usize,
    num_states: usize,
    traversals: FnvHashSet<SPSP>,
}

impl GadgetDef {
    /// Constructs the "nope" gadget
    pub fn new(num_states: usize, num_ports: usize) -> Self {
        Self {
            num_ports,
            num_states,
            traversals: FnvHashSet::default(),
        }
    }

    pub fn from_traversals<I: IntoIterator<Item = SPSP>>(
        num_states: usize,
        num_ports: usize,
        traversals: I,
    ) -> Self {
        Self {
            num_ports,
            num_states,
            traversals: traversals.into_iter().collect(),
        }
    }

    pub fn num_ports(&self) -> usize {
        self.num_ports
    }

    pub fn num_states(&self) -> usize {
        self.num_states
    }

    pub fn traversals(&self) -> impl Iterator<Item = &SPSP> {
        self.traversals.iter()
    }

    /// Gets all the destinations allowed in some state and port
    pub fn targets_from_state_port<'a>(&'a self, sp: SP) -> impl Iterator<Item = SP> + 'a {
        self.traversals
            .iter()
            .filter(move |((s, p), _)| *s == sp.0 && *p == sp.1)
            .map(move |(_, (s, p))| (*s, *p))
    }

    /// Gets all the port-to-port traversals allowed in some state
    pub fn port_traversals_in_state(&self, state: State) -> FnvHashSet<PP> {
        self.traversals
            .iter()
            .filter(|((s, _), _)| *s == state)
            .map(|((_, p0), (_, p1))| (*p0, *p1))
            .collect()
    }
}

pub struct Gadget {
    def: Rc<GadgetDef>,
    size: WH,
    /// Ports are located at midpoints of unit segments along the perimeter,
    /// starting from the bottom left and going counterclockwise.
    port_map: Vec<Option<Port>>,
    state: State,
    render: RefCell<GadgetRenderInfo>,
    dirty: Cell<bool>,
}

impl Gadget {
    /// Constructs a new `Gadget` with a gadget definition, a size,
    /// and a port map.
    ///
    /// Ports are located at midpoints of unit segments along the perimeter,
    /// starting from the bottom left and going counterclockwise. In the port map,
    /// a `None` represents the absence of a port.
    pub fn new(def: &Rc<GadgetDef>, size: WH, port_map: Vec<Option<Port>>, state: State) -> Self {
        let res = Self {
            def: Rc::clone(def),
            size,
            port_map,
            state,
            render: RefCell::new(GadgetRenderInfo::new()),
            dirty: Cell::new(true),
        };
        res
    }

    pub fn def(&self) -> &Rc<GadgetDef> {
        &self.def
    }

    pub fn size(&self) -> WH {
        self.size
    }

    pub fn port(&self, index: usize) -> Option<Port> {
        self.port_map[index]
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
        self.dirty.set(true);
    }

    fn port_map_inv(&self) -> FnvHashMap<Port, usize> {
        self.port_map
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_some())
            .map(|(i, p)| (p.unwrap(), i))
            .collect()
    }

    /// Gets the traversals allowed in the current state, at some port
    /// in back, right, front, left order relative to some facing direction
    pub fn targets_from_state_port_brfl(&self, port: Port, direction: XY) -> [Vec<SP>; 4] {
        let offset = if direction.x == 0 {
            if direction.y > 0 {
                0
            } else {
                2
            }
        } else {
            if direction.x > 0 {
                1
            } else {
                3
            }
        };

        let mut arr = [vec![], vec![], vec![], vec![]];
        let (w, h) = self.size();
        let map = self.port_map_inv();

        for sp in self.def().targets_from_state_port((self.state(), port)) {
            let (_, port) = sp;
            let idx = map[&port];

            if (idx as u32) < w + h {
                if (idx as u32) < w {
                    &mut arr[(0 + offset) % 4]
                } else {
                    &mut arr[(1 + offset) % 4]
                }
            } else {
                if (idx as u32) < w + h + w {
                    &mut arr[(2 + offset) % 4]
                } else {
                    &mut arr[(3 + offset) % 4]
                }
            }
            .push(sp);
        }

        arr
    }

    fn potential_port_positions(&self) -> Vec<Vec2> {
        (0..self.size.0)
            .map(|i| vec2(0.5 + i as f64, 0.0))
            .chain((0..self.size.1).map(|i| vec2(self.size.0 as f64, 0.5 + i as f64)))
            .chain(
                (0..self.size.0)
                    .rev()
                    .map(|i| vec2(0.5 + i as f64, self.size.1 as f64)),
            )
            .chain((0..self.size.1).rev().map(|i| vec2(0.0, 0.5 + i as f64)))
            .collect()
    }

    /// Rotates the ports of the gadget by some number of spaces.
    /// A positive number means counterclockwise,
    /// a negative number means clockwise.
    pub fn rotate_ports(&mut self, num_spaces: i32) {
        self.dirty.set(true);
        let rem = (-num_spaces).rem_euclid(self.port_map.len() as i32);

        let len = self.port_map.len();
        self.port_map = self
            .port_map
            .iter()
            .cycle()
            .skip(rem as usize)
            .take(len)
            .copied()
            .collect();
    }

    /// Temporary function to flip ports; in a hurry
    pub fn flip_ports(&mut self) {
        self.dirty.set(true);
        self.port_map.reverse();
    }

    /// Adds 1 to the state; resetting it to 0 in case of overflow
    pub fn cycle_state(&mut self) {
        self.dirty.set(true);
        self.set_state((self.state + 1) % self.def.num_states() as State);
    }

    /// Gets the positions of the ports of this gadget in port order.
    /// The positions are relative to the bottom-left corner.
    pub fn port_positions(&self) -> Vec<Vec2> {
        let mut vec = Vec::new();
        vec.resize(self.def.num_ports, vec2(0.0, 0.0));

        let x: f32 = 0.0;
        let y: f32 = 0.0;

        for (port, position) in self.port_map.iter().zip(self.potential_port_positions()) {
            if let Some(port) = port {
                vec[*port as usize] = position;
            }
        }

        vec
    }

    /// Updates the rendering information
    pub fn update_render(&self) {
        self.render.borrow_mut().update(self);
    }

    pub fn renderer(&self) -> Ref<GadgetRenderInfo> {
        if self.dirty.get() {
            self.dirty.set(false);
            self.update_render();
        }
        self.render.borrow()
    }
}

impl Clone for Gadget {
    fn clone(&self) -> Self {
        Self {
            def: Rc::clone(&self.def),
            size: self.size,
            port_map: self.port_map.clone(),
            state: self.state,
            render: self.render.clone(),
            dirty: self.dirty.clone(),
        }
    }
}

pub struct GadgetRenderInfo {
    /// 3 coordinates per position
    positions: Vec<f32>,
    /// 3 components per color
    colors: Vec<f32>,
    /// 3 indexes per triangle
    indexes: Vec<u32>,
    paths: FnvHashMap<PP, Path>,
    model: RefCell<Option<Model>>,
}

impl GadgetRenderInfo {
    pub const RECTANGLE_Z: f64 = -0.001;
    const OUTLINE_Z: f64 = -0.002;
    const PATH_Z: f64 = -0.003;
    const PORT_Z: f64 = -0.004;

    fn new() -> Self {
        Self {
            positions: vec![],
            colors: vec![],
            indexes: vec![],
            paths: FnvHashMap::default(),
            model: RefCell::new(None),
        }
    }

    fn has_outline(&self, gadget: &Gadget) -> bool {
        gadget.def().num_states() > 1
    }

    /// Gets the path a robot takes to go from p0 to p1
    fn port_path(ports: PP, port_positions: &Vec<Vec2>) -> Path {
        let positions = [
            port_positions[ports.0 as usize],
            port_positions[ports.1 as usize],
        ];
        let mut bezier = [vec2(0.0, 0.0), vec2(0.0, 0.0)];

        let offset = 0.25;

        for (pos, bez) in positions.iter().zip(bezier.iter_mut()) {
            *bez = pos
                + if pos.x.floor() == pos.x {
                    // on vertical edge
                    if pos.x == 0.0 {
                        // on left edge
                        vec2(offset, 0.0)
                    } else {
                        // on right edge
                        vec2(-offset, 0.0)
                    }
                } else {
                    // on horizontal edge
                    if pos.y == 0.0 {
                        // on bottom edge
                        vec2(0.0, offset)
                    } else {
                        // on top edge
                        vec2(0.0, -offset)
                    }
                }
        }

        // Same-port traversal; make it look like a loop
        if bezier[0] == bezier[1] {
            let dv = (bezier[0] - positions[0]).right_ccw();

            bezier[0] += dv;
            bezier[1] -= dv;
        }

        Path::from_bezier3(
            [positions[0], bezier[0], bezier[1], positions[1]],
            GadgetRenderInfo::PATH_Z,
            0.05,
        )
    }

    /// Updates the rendering information so
    /// that it is correct when rendering
    fn update(&mut self, gadget: &Gadget) {
        self.positions.clear();
        self.colors.clear();
        self.indexes.clear();
        self.paths.clear();
        *self.model.borrow_mut() = None;

        // Surrounding rectangle
        let rect = Rectangle::new(
            0.0,
            gadget.size().0 as f64,
            0.0,
            gadget.size().1 as f64,
            GadgetRenderInfo::RECTANGLE_Z,
        );
        rect.append_to(&mut self.positions, &mut self.indexes);
        self.colors.extend(&[
            0.6, 0.8, 1.0, 1.0, 0.7, 0.9, 1.0, 1.0, 0.9, 1.0, 1.0, 1.0, 0.8, 1.0, 1.0, 1.0,
        ]);

        // Port circles
        let port_positions = gadget.port_positions();
        for vec in port_positions.iter() {
            let circle = Circle::new(vec.x, vec.y, GadgetRenderInfo::PORT_Z, 0.05);
            circle.append_to(&mut self.positions, &mut self.indexes);
            self.colors.extend(
                [0.0, 0.0, 0.75, 1.0]
                    .iter()
                    .cycle()
                    .take(circle.num_vertices() * 4),
            );
        }

        // Outline
        if self.has_outline(gadget) {
            let path = Path::new(
                vec![
                    vec2(0.0, 0.0),
                    vec2(0.0, gadget.size().1 as f64),
                    vec2(gadget.size().0 as f64, gadget.size().1 as f64),
                    vec2(gadget.size().0 as f64, 0.0),
                ],
                GadgetRenderInfo::OUTLINE_Z,
                0.05,
                true,
            );
            path.append_to(&mut self.positions, &mut self.indexes);
            self.colors.extend(
                [0.0, 0.0, 0.0, 1.0]
                    .iter()
                    .cycle()
                    .take(path.num_vertices() * 4),
            );
        }

        // Paths
        for ports in gadget.def().port_traversals_in_state(gadget.state()) {
            let path = GadgetRenderInfo::port_path(ports, &port_positions);

            self.paths.insert(ports, path);
        }

        for ((p0, p1), path) in &self.paths {
            let directed = self.paths.get(&(*p1, *p0)).is_none();

            // No redundant path drawing!
            if p0 <= p1 || directed {
                path.append_to(&mut self.positions, &mut self.indexes);
                self.colors.extend(
                    [0.0, 0.0, 0.0, 1.0]
                        .iter()
                        .cycle()
                        .take(path.num_vertices() * 4),
                );
            }

            if directed {
                let dir = path.end_direction();
                let end = port_positions[*p1 as usize];

                let v0 = end + dir * -0.2 + dir.right_ccw() * -0.1;
                let v2 = end + dir * -0.2 + dir.right_ccw() * 0.1;

                let old_len = self.positions().len() as u32 / 3;
                #[rustfmt::skip]
                self.positions.extend(&[
                    v0.x as f32, v0.y as f32, GadgetRenderInfo::PATH_Z as f32,
                    end.x as f32, end.y as f32, GadgetRenderInfo::PATH_Z as f32,
                    v2.x as f32, v2.y as f32, GadgetRenderInfo::PATH_Z as f32,
                ]);
                self.colors
                    .extend([0.0, 0.0, 0.0, 1.0].iter().cycle().take(4 * 3));
                self.indexes.extend(&[old_len, old_len + 1, old_len + 2]);
            }
        }
    }

    pub fn colors(&self) -> &Vec<f32> {
        &self.colors
    }

    pub fn model(&self, gl: &Rc<Context>) -> Ref<Model> {
        {
            let mut model = self.model.borrow_mut();

            if model.is_none() {
                *model = Some(Model::new(gl, &self.positions, &self.colors, &self.indexes));
            }
        }
        Ref::map(self.model.borrow(), |m| m.as_ref().unwrap())
    }
}

impl Clone for GadgetRenderInfo {
    fn clone(&self) -> Self {
        Self {
            positions: self.positions.clone(),
            colors: self.colors.clone(),
            indexes: self.indexes.clone(),
            paths: self.paths.clone(),
            model: RefCell::new(None),
        }
    }
}

impl Shape for GadgetRenderInfo {
    fn num_vertices(&self) -> usize {
        self.positions().len() / 3
    }

    fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }

    fn indexes(&self) -> Vec<u32> {
        self.indexes.clone()
    }
}

/// Walks around in a maze of gadgets
pub struct Agent {
    /// Double the position, because then it's integers
    double_xy: XY,
    /// either (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), or (0.0, -1.0)
    direction: Vec2i,
    /// rendering, of course
    model: Rc<Model>,
}

impl Agent {
    pub fn new(position: Vec2, direction: Vec2i, model: &Rc<Model>) -> Self {
        let double_xy = vec2(
            (position.x * 2.0).round() as i32,
            (position.y * 2.0).round() as i32,
        );

        Self {
            double_xy,
            direction,
            model: Rc::clone(model),
        }
    }

    pub fn set_position(&mut self, position: Vec2) {
        self.double_xy = vec2(
            (position.x * 2.0).round() as i32,
            (position.y * 2.0).round() as i32,
        );
    }

    pub fn rotate(&mut self, num_right_turns: i32) {
        for _ in 0..(num_right_turns.rem_euclid(4)) {
            self.direction = self.direction.right_ccw();
        }
    }

    /// Advances the agent according to internal rules
    pub fn advance(&mut self, grid: &mut Grid<Gadget>, input: Vec2i) {
        if input.dot_ex(self.direction) == -1 {
            // Turn around, that's it
            self.direction *= -1;
            return;
        }

        if let Some((gadget, xy, (w, h), idx)) =
            grid.get_item_touching_edge_mut(self.double_xy, self.direction)
        {
            if let Some(port) = gadget.port(idx) {
                let [back, right, front, left] =
                    gadget.targets_from_state_port_brfl(port, self.direction);

                // TODO: Make this more sophisticated; don't just take the first traversal

                let sp;

                if input.dot_ex(self.direction) == 1 {
                    // Forward
                    sp = front
                        .first()
                        .or_else(|| left.first().xor(right.first()))
                        .or_else(|| back.first());
                } else if self.direction.right_ccw() == input {
                    // Left
                    sp = left.first();
                } else if input.dot_ex(self.direction) == -1 {
                    // Back
                    // TODO: Unreachable right now
                    sp = None;
                } else {
                    // Right
                    sp = right.first();
                }

                if let Some((s1, p1)) = sp {
                    let pos2 = (gadget.port_positions()[*p1 as usize] * 2.0)
                        .cast::<i32>()
                        .unwrap();
                    self.direction = if pos2.x.rem_euclid(2) != 0 {
                        if pos2.y == 0 {
                            // Bottom
                            vec2(0, -1)
                        } else {
                            // Top
                            vec2(0, 1)
                        }
                    } else {
                        if pos2.x == 0 {
                            // Left
                            vec2(-1, 0)
                        } else {
                            // Right
                            vec2(1, 0)
                        }
                    };

                    self.double_xy = xy * 2 + pos2;
                    gadget.set_state(*s1);
                }
            }
        }
    }

    pub fn render(&self, camera: &Camera) {
        let dir = self.direction.cast::<f64>().unwrap();

        let transform = Mat4::from_cols(
            -dir.right_ccw().extend(0.0).extend(0.0),
            dir.extend(0.0).extend(0.0),
            vec4(0.0, 0.0, 1.0, 0.0),
            (self.double_xy.cast::<f64>().unwrap() * 0.5)
                .extend(-0.1)
                .extend(1.0),
        );
        self.model.render(transform, camera);
    }

    /// Returns the model that an agent uses
    pub fn new_shared_model(gl: &Rc<Context>) -> Model {
        let positions: Vec<f32> = vec![
            0.15, -0.15, 0.0, 0.15, 0.0, 0.0, 0.0, 0.15, 0.0, -0.15, 0.0, 0.0, -0.15, -0.15, 0.0,
        ];
        let colors: Vec<f32> = vec![
            0.0, 0.8, 0.0, 1.0, 0.0, 0.6, 0.0, 1.0, 0.0, 0.4, 0.0, 1.0, 0.0, 0.6, 0.0, 1.0, 0.0,
            0.8, 0.0, 1.0,
        ];
        let indexes: Vec<u32> = vec![0, 1, 2, 0, 2, 4, 2, 3, 4];

        Model::new(gl, &positions, &colors, &indexes)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_nope() {
        let def = GadgetDef::new(4, 3);
        assert_eq!(3, def.num_ports());
        assert_eq!(4, def.num_states());
        assert_eq!(0, def.traversals().count());
    }

    #[test]
    fn test_from_traversals() {
        let def = GadgetDef::from_traversals(2, 2, vec![((0, 0), (1, 1)), ((1, 1), (0, 0))]);
        assert_eq!(2, def.num_ports());
        assert_eq!(2, def.num_states());

        let mut expected = FnvHashSet::default();
        expected.insert(((0, 0), (1, 1)));
        expected.insert(((1, 1), (0, 0)));
        let result = def.traversals().copied().collect::<FnvHashSet<_>>();
        assert_eq!(result, expected);
    }
}
