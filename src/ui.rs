use cgmath::vec2;
use conrod_core::color;
use conrod_core::position::{Align, Place, Relative};
use conrod_core::render::PrimitiveWalker;
use conrod_core::widget::text::{self, Text};
use conrod_core::widget::Canvas;
use conrod_core::widget::{self, bordered_rectangle, matrix, BorderedRectangle, List, Matrix};
use conrod_core::widget_ids;
use conrod_core::{Borderable, Color, Colorable, Positionable, Sizeable, Theme, Widget};
use conrod_core::{Ui, UiCell};
use ref_thread_local::RefThreadLocal;

use crate::gadget::Agent;
use crate::log;
use crate::math::Vec2i;
use crate::render::{Model, ModelType, ShaderType, TrianglesEx, TrianglesType, MODELS};
use crate::render::{SHADERS, TRIANGLESES};
use crate::widget::button;
use crate::widget::screen::SelectFunc;
use crate::widget::{screen, Button, ContraptionScreen, SelectionGrid, Triangles3d};
use crate::App;
use crate::UndoAction;

widget_ids! {
    pub struct WidgetIds {
        contraption_screen, menu, menu_list, gadget_select, agent, version,
        canvas, header, body, left_sidebar,
    }
}

pub fn theme() -> Theme {
    Theme {
        background_color: color::TRANSPARENT,
        shape_color: color::TRANSPARENT,
        border_color: color::TRANSPARENT,
        ..Theme::default()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
// Mode of editing
pub enum Mode {
    None,
    TilePaint,
    AgentPlace,
    Play,
    Select,
    GadgetPaste,
}

impl<'a> App<'a> {
    pub fn set_mode(&mut self, mode: Mode) {
        if mode != self.mode {
            // clear some fields
            if mode != Mode::TilePaint {
                self.gadget_selection = None;
                self.gadget_tile = None;
            }

            // Play time! Use the playing undo stack
            if mode == Mode::Play {
                self.undo_stack_index = 1;
                self.undo_stack_mut().clear();
            }

            // Play time's over! Move the entire play history to the main stack as a single batch
            if self.mode == Mode::Play {
                self.undo_stack_index = 0;

                let (main_stack, play_stack) = self.undo_stacks.split_at_mut(1);
                let main_stack = &mut main_stack[0];
                let play_stack = &mut play_stack[0];

                main_stack
                    .as_mut()
                    .expect("Tried to get undo stack while undoing/redoing")
                    .append_as_batch(
                        play_stack
                            .as_mut()
                            .expect("Tried to get undo stack while undoing/redoing"),
                    );
            }

            if mode != Mode::AgentPlace && mode != Mode::Play {
                self.agent = None;
            }

            if mode != Mode::Select {
                self.selection.clear();
            }

            if self.mode == Mode::GadgetPaste {
                // Just in case a cut was performed without a paste
                self.undo_stack_mut().batch();
            }

            self.mode = mode;
        }
    }

    pub fn update_ui(&mut self, ui: &mut Ui) {
        let mut ui = ui.set_widgets();

        // Contraption screen
        for event in ContraptionScreen::new(self.mode, &self.camera)
            .middle_of(ui.window)
            .wh_of(ui.window)
            //.x_y(0.0, 0.0)
            //.wh_of(ui.window)
            .set(self.ids.contraption_screen, &mut ui)
        {
            match event {
                screen::Event::TilePaint(xy) => {
                    if let Some(gadget) = &self.gadget_tile {
                        // Nope gadget is special
                        if gadget.def().num_states() == 1
                            && gadget.def().num_ports() == 0
                            && gadget.size() == (1, 1)
                        {
                            self.remove_gadget_from_grid(xy);
                        } else {
                            self.add_gadget_to_grid(gadget.clone(), xy);
                        }
                        crate::save_grid_in_url(&self.grid);
                    }
                }

                screen::Event::TileHover(xy) => {
                    self.gadget_tile_xy = xy;
                }

                screen::Event::TilePaintFinish => {
                    self.undo_stack_mut().batch();
                }

                screen::Event::AgentPlace(xy) => {
                    if self.agent.is_some() {
                        self.set_mode(Mode::Play);
                    }

                    if let Some(agent) = &mut self.agent {
                        agent.set_position(xy);
                    }
                }

                screen::Event::AgentHover(xy) => {
                    if let Some(agent) = &mut self.agent {
                        agent.set_position(xy);
                    }
                }

                screen::Event::Pan(xy) => {
                    self.center += xy;
                }

                screen::Event::Zoom(xy, amount) => {
                    let prev_height = self.height;
                    self.height += amount;
                    self.clamp_height(&ui);
                    self.center = xy + (self.center - xy) * self.height / prev_height;
                }

                screen::Event::Select(rect, func) => {
                    let (l, r, b, t) = rect.l_r_b_t();

                    let selection = self
                        .grid
                        .get_in_bounds(l, r, b, t)
                        .map(|(_, xy, wh)| (*xy, *wh));

                    match func {
                        SelectFunc::Replace => self.selection = selection.collect(),

                        SelectFunc::Add => self.selection.extend(selection),

                        SelectFunc::Subtract => {
                            self.selection = self
                                .selection
                                .difference(&selection.collect())
                                .copied()
                                .collect()
                        }

                        SelectFunc::Xor => {
                            self.selection = self
                                .selection
                                .symmetric_difference(&selection.collect())
                                .copied()
                                .collect()
                        }
                    }
                }

                screen::Event::GadgetPaste(xy) => {
                    for (t, xy, _) in self.paste.clone().translate(xy) {
                        self.add_gadget_to_grid(t, xy);
                    }
                    self.undo_stack_mut().batch();
                }

                screen::Event::GadgetPasteHover(xy) => {
                    self.paste_xy = xy;
                }

                screen::Event::MousePosition(position) => {
                    self.grid_mouse_position = position;
                }
            }
        }

        let new_canvas = || Canvas::new().graphics_for(self.ids.contraption_screen);

        new_canvas()
            .flow_down(&[
                (self.ids.header, new_canvas().length(40.0)),
                (
                    self.ids.body,
                    new_canvas().flow_right(&[(self.ids.left_sidebar, new_canvas().length(260.0))]),
                ),
            ])
            .set(self.ids.canvas, &mut ui);

        // Menu
        BorderedRectangle::new([1.0, 1.0])
            .with_style(bordered_rectangle::Style {
                color: Some(Color::Rgba(0.9, 0.9, 0.9, 1.0)),
                border: None,
                border_color: Some(color::BLACK),
            })
            .middle_of(self.ids.header)
            .wh_of(self.ids.header)
            .set(self.ids.menu, &mut ui);

        let (mut items, _) = List::flow_right(5)
            .middle_of(self.ids.menu)
            .wh_of(self.ids.menu)
            .set(self.ids.menu_list, &mut ui);

        // lifetimes in closures when
        fn as_menu_button<'a>(
            button: Button<'a, button::Triangles>,
            this: &mut App,
            ui: &mut UiCell,
        ) -> Button<'a, button::Triangles> {
            let height = ui.h_of(this.ids.menu_list).expect("No menu list!");

            button.padding(3.0).w(height).h_of(this.ids.menu_list)
        }

        for _ in items.next(&ui).unwrap().set(
            as_menu_button(
                Button::triangles(Triangles3d::from_gadget(&self.gadget_select_rep)),
                self,
                &mut ui,
            )
            .tooltip_text("Select gadget"),
            &mut ui,
        ) {
            self.set_mode(Mode::TilePaint);
        }

        for _ in items.next(&ui).unwrap().set(
            as_menu_button(
                Button::triangles(Triangles3d::new(
                    (*TRIANGLESES.borrow()[TrianglesType::Agent])
                        .clone()
                        .with_default_extra(),
                    vec2(0.0, 0.0),
                    0.3,
                    0.3,
                )),
                self,
                &mut ui,
            )
            .tooltip_text("Place agent"),
            &mut ui,
        ) {
            self.set_mode(Mode::AgentPlace);
            self.agent = Some(Agent::new(vec2(0.5, 0.0), vec2(0, 1)));
        }

        for _ in items.next(&ui).unwrap().set(
            as_menu_button(
                Button::triangles(Triangles3d::new(
                    (*TRIANGLESES.borrow()[TrianglesType::Select])
                        .clone()
                        .with_default_extra(),
                    vec2(0.0, 0.0),
                    2.0,
                    2.0,
                )),
                self,
                &mut ui,
            )
            .tooltip_text("Select"),
            &mut ui,
        ) {
            self.set_mode(Mode::Select);
        }

        for _ in items.next(&ui).unwrap().set(
            as_menu_button(
                Button::triangles(Triangles3d::new(
                    (*TRIANGLESES.borrow()[TrianglesType::Undo])
                        .clone()
                        .with_default_extra(),
                    vec2(0.0, 0.0),
                    2.0,
                    2.0,
                )),
                self,
                &mut ui,
            )
            .tooltip_text("Undo"),
            &mut ui,
        ) {
            self.undo();
        }

        for _ in items.next(&ui).unwrap().set(
            as_menu_button(
                Button::triangles(Triangles3d::new(
                    (*TRIANGLESES.borrow()[TrianglesType::Undo])
                        .clone()
                        .with_default_extra(),
                    vec2(0.0, 0.0),
                    -2.0,
                    2.0,
                )),
                self,
                &mut ui,
            )
            .tooltip_text("Redo"),
            &mut ui,
        ) {
            self.redo();
        }

        // Gadget selector
        if self.mode != Mode::Play {
            let selection = SelectionGrid::new(4, &self.gadget_select, self.gadget_selection)
                .color(Color::Rgba(0.8, 0.9, 0.8, 1.0))
                .border_color(color::BLACK)
                .outer_padding(5.0)
                .middle_of(self.ids.left_sidebar)
                .padded_wh_of(self.ids.left_sidebar, 10.0)
                .set(self.ids.gadget_select, &mut ui);

            if let Some(selection) = selection {
                self.set_mode(Mode::TilePaint);
                self.gadget_selection = Some(selection);

                let gadget = self.gadget_select[selection].clone();
                self.gadget_tile = Some(gadget);
            }
        }

        // Version number
        Text::new("Gadget Up! 2 v0.3.0")
            .font_size(12)
            .bottom_left_with_margin_on(self.ids.gadget_select, 3.0)
            .set(self.ids.version, &mut ui);
    }

    pub fn render_ui(&mut self, ui: &mut Ui, width: f64, height: f64) {
        self.ui_renderer.draw_begin(width, height);

        let mut primitives = ui.draw();
        while let Some(primitive) = PrimitiveWalker::next_primitive(&mut primitives) {
            self.ui_renderer.primitive(primitive, ui);
        }

        self.ui_renderer.draw_end();
    }
}
