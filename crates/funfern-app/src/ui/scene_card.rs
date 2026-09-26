//! The card in the viewport's top-left corner while a gallery example is
//! open: which one it is, what it shows, and the way on to the next.

use bevy_egui::egui::{self, Rect};
use funfern_app::topology_examples::catalog;

use super::*;

/// What the viewer has done with the card this session. Folding it holds
/// across examples, so stepping through them folded keeps it folded; closing
/// it holds only until another example opens.
pub(super) struct SceneCard {
    pub(super) expanded: bool,
    pub(super) dismissed: bool,
    /// The line a first launch adds, pointing at the rest of the gallery. It
    /// goes once the viewer has stepped, opened the gallery or closed the card.
    pub(super) hint: bool,
}

impl Default for SceneCard {
    fn default() -> Self {
        Self {
            expanded: true,
            dismissed: false,
            hint: false,
        }
    }
}

/// The widest the card grows, which is what an expanded card's description
/// wraps to.
const CARD_WIDTH: f32 = 340.0;
const CARD_INSET: f32 = 12.0;

impl Playground {
    /// The catalog entry the card shows, if it shows one.
    pub(super) fn scene_card_example(&self) -> Option<usize> {
        self.example_opened.filter(|_| !self.scene_card.dismissed)
    }

    /// Opens the example `offset` places along the catalog from the open one,
    /// round from the last to the first and back.
    pub(super) fn step_example(&mut self, offset: isize) {
        let Some(index) = self.example_opened else {
            return;
        };
        let total = catalog().len() as isize;
        self.open_example((index as isize + offset).rem_euclid(total) as usize);
    }

    /// Draws the card over the viewport's top-left corner and acts on it.
    /// Returns where it was drawn.
    pub(super) fn scene_card(&mut self, ctx: &egui::Context, viewport: Rect) -> Option<Rect> {
        if self.examples_open {
            self.scene_card.hint = false;
        }
        let index = self.scene_card_example()?;
        let catalog = catalog();
        let example = &catalog[index];
        let width = CARD_WIDTH.min(viewport.width() - 2.0 * CARD_INSET);
        let mut step = 0;
        let mut toggle = false;
        let mut close = false;
        let mut gallery = false;
        let expanded = self.scene_card.expanded;
        let hint = self.scene_card.hint;
        let card = egui::Area::new(egui::Id::new("scene_card"))
            .fixed_pos(viewport.left_top() + egui::vec2(CARD_INSET, CARD_INSET))
            .constrain_to(viewport)
            .show(ctx, |ui| {
                egui::Frame::window(ui.style()).show(ui, |ui| {
                    // A fixed width, so the buttons stay where they are while
                    // the viewer steps from one name to the next.
                    ui.set_width(width);
                    ui.horizontal(|ui| {
                        if ui
                            .small_button("⏴")
                            .on_hover_text("Previous example")
                            .clicked()
                        {
                            step = -1;
                        }
                        ui.weak(format!("{}/{}", index + 1, catalog.len()));
                        if ui.small_button("⏵").on_hover_text("Next example").clicked() {
                            step = 1;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("×").on_hover_text("Close").clicked() {
                                close = true;
                            }
                            let (fold, fold_hint) = if expanded {
                                ("⏶", "Hide the description")
                            } else {
                                ("⏷", "Show the description")
                            };
                            if ui.small_button(fold).on_hover_text(fold_hint).clicked() {
                                toggle = true;
                            }
                        });
                    });
                    ui.strong(example.name);
                    if expanded {
                        ui.small(example.group.label());
                        ui.label(example.description);
                        if hint {
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "One of {} examples: ⏴ ⏵ steps through them, and \
                                     Examples in the top bar shows them all.",
                                    catalog.len()
                                ))
                                .color(GOLD),
                            );
                        }
                        ui.add_space(2.0);
                        if ui.button("All examples…").clicked() {
                            gallery = true;
                        }
                    }
                });
            });
        if toggle {
            self.scene_card.expanded = !expanded;
        }
        if close {
            self.scene_card.dismissed = true;
            self.scene_card.hint = false;
        }
        if gallery {
            self.examples_open = true;
            self.scene_card.hint = false;
        }
        if step != 0 {
            self.step_example(step);
        }
        Some(card.response.rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_support::*;
    use bevy_egui::egui::Pos2;

    fn index_of(name: &str) -> usize {
        catalog()
            .iter()
            .position(|example| example.name == name)
            .unwrap()
    }

    #[test]
    fn stepping_wraps_round_the_catalog() {
        let mut state = Playground::default();
        let last = catalog().len() - 1;
        state.open_example(0);
        state.step_example(-1);
        assert_eq!(state.example_opened, Some(last));
        assert_eq!(state.editor.document.model, catalog()[last].document.model);
        state.step_example(1);
        assert_eq!(state.example_opened, Some(0));
        state.step_example(1);
        assert_eq!(state.example_opened, Some(1));
    }

    #[test]
    fn a_closed_card_stays_closed_until_another_example_opens() {
        let mut state = Playground::default();
        state.open_example(index_of("Double slit"));
        state.scene_card.dismissed = true;
        assert_eq!(state.scene_card_example(), None);
        state.open_example(index_of("Talbot carpet"));
        assert_eq!(state.scene_card_example(), Some(index_of("Talbot carpet")));
    }

    /// The descriptions ask for edits, so an edit keeps the card; a scene
    /// that is not the example any more, however it came, has none.
    #[test]
    fn an_edit_keeps_the_card_and_a_replaced_scene_drops_it() {
        let mut state = Playground::default();
        let index = index_of("Bent fiber");
        state.open_example(index);
        settle(&mut state.editor);
        state
            .editor
            .set_domain(DomainRect {
                max_x: 1.4,
                ..DomainRect::UNIT
            })
            .unwrap();
        assert_eq!(state.scene_card_example(), Some(index));
        state.new_scene();
        assert_eq!(state.scene_card_example(), None);
        state.undo();
        assert_eq!(state.scene_card_example(), None, "undo is not an example");
    }

    #[test]
    fn a_first_launch_hints_at_the_gallery_until_the_viewer_moves_on() {
        let mut state = Playground::default();
        state.open_random_example();
        assert!(state.scene_card.hint && state.scene_card.expanded);
        assert!(state.scene_card_example().is_some());
        state.step_example(1);
        assert!(!state.scene_card.hint, "stepping kept the hint");

        state.open_random_example();
        let context = egui::Context::default();
        state.examples_open = true;
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            state.scene_card(ui.ctx(), viewport());
        });
        assert!(!state.scene_card.hint, "the gallery kept the hint");
    }

    /// One frame laid out as the app lays it out, a viewport-filling painter
    /// under the card, and whether the viewport took a click.
    fn frame(
        state: &mut Playground,
        context: &egui::Context,
        events: Vec<egui::Event>,
    ) -> (bool, Option<Rect>) {
        let input = egui::RawInput {
            screen_rect: Some(viewport()),
            events,
            ..egui::RawInput::default()
        };
        let mut clicked = false;
        let mut card = None;
        let _ = context.run_ui(input, |ui| {
            let mut rect = Rect::NOTHING;
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let (response, _) =
                        ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
                    clicked = response.clicked();
                    rect = response.rect;
                });
            card = state.scene_card(ui.ctx(), rect);
        });
        (clicked, card)
    }

    fn viewport_clicked_at(state: &mut Playground, context: &egui::Context, at: Pos2) -> bool {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (pressed, _) = frame(
            state,
            context,
            vec![egui::Event::PointerMoved(at), button(true)],
        );
        let (released, _) = frame(state, context, vec![button(false)]);
        pressed || released
    }

    #[test]
    fn a_click_on_the_card_stays_off_the_viewport() {
        let mut state = Playground::default();
        let context = egui::Context::default();
        let mut card = None;
        for _ in 0..3 {
            card = frame(&mut state, &context, vec![]).1;
        }
        let card = card.expect("no card over an example");
        // The description, which is no button: only the card is there.
        assert!(!viewport_clicked_at(&mut state, &context, card.center()));
        assert_eq!(state.example_opened, Some(0), "the click did something");
        let beside = card.right_bottom() + egui::vec2(40.0, 40.0);
        assert!(viewport_clicked_at(&mut state, &context, beside));
    }

    #[test]
    fn the_cards_glyphs_are_in_the_fonts() {
        let context = egui::Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |_| {});
        let font = egui::FontId::proportional(14.0);
        assert!(context.fonts_mut(|fonts| fonts.has_glyphs(&font, "⏴⏵⏶⏷×…")));
    }
}
