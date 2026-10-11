//! Detachable mask list and component selection. Layout is a UI preference, never a photo edit.
use super::*;
use serde_json::Value;

pub fn command(app: &mut LightcraftApp, command: &str, params: &Value) -> Result<Value, String> {
    match command {
        "ui.maskPanel" => {
            let floating = params.get("floating").and_then(Value::as_bool).ok_or("floating must be a boolean")?;
            app.ui.mask_panel_floating = floating;
            Ok(json!({"floating": floating}))
        }
        "mask.selectComponent" => {
            let photo = app.session.active().ok_or("No photo selected")?;
            let mask = match params.get("id") {
                Some(value) => value.as_u64().and_then(|v| u32::try_from(v).ok()).ok_or("id must be a mask ID")?,
                None => app.session.active_mask.ok_or("No mask selected")?,
            };
            let component =
                params.get("component").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or("component must be an index")?;
            let d = app.session.develop_of(photo).unwrap_or_default();
            let shape = &d.masks.iter().find(|m| m.id == mask).and_then(|m| m.components.get(component)).ok_or("No such mask component")?.shape;
            let tool = tool_for(shape);
            app.session.active_mask = Some(mask);
            app.ui.mask_component = Some((photo, mask, component));
            app.ui.tool = tool.into();
            Ok(json!({"mask": mask, "component": component}))
        }
        "mask.brushSettings" => {
            let number = |key: &str, current: f32, lo: f32, hi: f32| -> Result<f32, String> {
                match params.get(key) {
                    None => Ok(current),
                    Some(v) => v
                        .as_f64()
                        .filter(|v| v.is_finite() && (*v >= lo as f64 && *v <= hi as f64))
                        .map(|v| v as f32)
                        .ok_or_else(|| format!("{key} must be between {lo} and {hi}")),
                }
            };
            let flag = |key: &str, current: bool| -> Result<bool, String> {
                params.get(key).map_or(Ok(current), |v| v.as_bool().ok_or_else(|| format!("{key} must be a boolean")))
            };
            let size = number("size", app.ui.brush_size, 0.0025, 0.25)?;
            let feather = number("feather", app.ui.brush_feather, 0.0, 100.0)?;
            let flow = number("flow", app.ui.brush_flow, 1.0, 100.0)?;
            let density = number("density", app.ui.brush_density, 0.0, 100.0)?;
            let erase = flag("erase", app.ui.brush_erase)?;
            let auto_mask = flag("autoMask", app.ui.brush_auto_mask)?;
            app.ui.brush_size = size;
            app.ui.brush_feather = feather;
            app.ui.brush_flow = flow;
            app.ui.brush_density = density;
            app.ui.brush_erase = erase;
            app.ui.brush_auto_mask = auto_mask;
            Ok(json!({"size": size, "feather": feather, "flow": flow, "density": density, "erase": erase, "autoMask": auto_mask}))
        }
        _ => Err(format!("Unknown masking UI command: {command}")),
    }
}

fn tool_for(shape: &MaskShape) -> &'static str {
    match shape {
        MaskShape::Brush { .. } => "brush",
        MaskShape::Linear { .. } => "linear",
        MaskShape::Radial { .. } => "radial",
        MaskShape::ColorRange { .. } => "colorRange",
        MaskShape::Object { .. } => "object",
        _ => "",
    }
}

pub fn component(app: &LightcraftApp, mask: &lightcraft_develop::Mask) -> Option<usize> {
    app.ui
        .mask_component
        .filter(|(photo, mid, i)| Some(*photo) == app.session.active() && *mid == mask.id && *i < mask.components.len())
        .map(|(_, _, i)| i)
        .or_else(|| mask.components.len().checked_sub(1))
}

/// A canvas tool only targets the selected component when its kind agrees with that tool.
pub fn tool_component(app: &LightcraftApp, tool: &str) -> Option<usize> {
    let d = app.session.develop_of(app.session.active()?)?;
    let m = d.masks.iter().find(|m| Some(m.id) == app.session.active_mask)?;
    let i = component(app, m)?;
    (tool_for(&m.components.get(i)?.shape) == tool).then_some(i)
}

pub fn selection_changed(app: &mut LightcraftApp, command: &str, params: &Value) {
    if !matches!(
        command,
        "mask.add" | "mask.addComponent" | "mask.select" | "mask.delete" | "mask.duplicate" | "mask.component" | "edit.undo" | "edit.redo"
    ) {
        return;
    }
    if matches!(command, "edit.undo" | "edit.redo") && app.ui.right != crate::state::RightPanel::Masking {
        return;
    }
    let action = params.get("action").and_then(Value::as_str);
    if command == "mask.component" && !matches!(action, Some("delete" | "duplicate")) {
        return;
    }
    let Some(photo) = app.session.active() else { return };
    let d = app.session.develop_of(photo).unwrap_or_default();
    let target = if command == "mask.addComponent" {
        params.get("id").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()).or(app.session.active_mask)
    } else {
        app.session.active_mask
    };
    let Some(m) = d.masks.iter().find(|m| Some(m.id) == target) else {
        app.ui.mask_component = None;
        if app.ui.right == crate::state::RightPanel::Masking {
            app.ui.tool.clear();
        }
        return;
    };
    let previous = app.ui.mask_component.filter(|(pid, mid, _)| *pid == photo && *mid == m.id).map(|(_, _, i)| i);
    let changed = params.get("component").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok());
    let preferred = match command {
        "mask.add" | "mask.addComponent" | "mask.duplicate" => None,
        "mask.component" => match (action, changed, previous) {
            (Some("duplicate"), Some(i), _) => i.checked_add(1),
            (Some("delete"), Some(i), Some(selected)) if i < selected => selected.checked_sub(1),
            _ => previous,
        },
        _ => previous,
    };
    let Some(last) = m.components.len().checked_sub(1) else { return };
    let _ = command_select(app, m.id, preferred.unwrap_or(last).min(last));
}

fn command_select(app: &mut LightcraftApp, mask: u32, component: usize) -> Result<Value, String> {
    command(app, "mask.selectComponent", &json!({"id": mask, "component": component}))
}

pub fn floating(app: &mut LightcraftApp, ctx: &egui::Context) {
    if !app.ui.mask_panel_floating || app.ui.right != crate::state::RightPanel::Masking || !crate::docking::exposed(app, crate::docking::Panel::Tools)
    {
        return;
    }
    let Some(photo) = app.session.active() else { return };
    let d = app.session.develop_of(photo).unwrap_or_default();
    if d.masks.is_empty() {
        return;
    }
    let bounds = ctx.content_rect();
    let right = app.widgets.iter().find(|(id, _)| id == "panel:right_panel").map_or(bounds.right() - app.ui.right_width, |(_, r)| r.left());
    let default = [right - 312.0, bounds.top() + 80.0, 296.0, 390.0];
    let [x, y, w, h] =
        app.ui.mask_panel_rect.filter(|rect| rect.iter().all(|v| v.is_finite()) && rect[2] >= 240.0 && rect[3] >= 180.0).unwrap_or(default);
    let size = vec2(w.clamp(240.0, 520.0).min(bounds.width()), h.clamp(180.0, 800.0).min(bounds.height()));
    let pos = pos2(
        x.clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left())),
        y.clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top())),
    );
    let mut open = true;
    let result = egui::Window::new(crate::i18n::tr("Masks"))
        .id(egui::Id::new("mask-list-window"))
        .current_pos(pos)
        .default_size(size)
        .min_size(vec2(240.0, 180.0))
        .max_size(vec2(520.0, 800.0))
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            list(app, ui, &d, true);
        });
    if let Some(result) = result {
        let r = result.response.rect;
        register(ctx, "panel:maskFloating", r);
        register(ctx, "maskPanelTitle", Rect::from_min_size(r.min, vec2((r.width() - 32.0).max(1.0), 26.0)));
        app.ui.mask_panel_rect = Some([r.left(), r.top(), r.width(), r.height()]);
    }
    if !open {
        let _ = app.run("ui.maskPanel", json!({"floating": false}));
    }
}

/// The list is shared by the embedded and floating layouts; component controls stay on the right.
pub(super) fn list(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &lightcraft_develop::DevelopSettings, floating: bool) {
    let t = Tokens::get(ui.ctx());
    let response = ui.scope(|ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Masks")).font(t.semibold(13.0)).color(t.text_label));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let plus = icon_button(ui, "maskCreate", Icon::Plus, vec2(24.0, 24.0), false, true, "Create New Mask");
                egui::Popup::menu(&plus).show(|ui| create_menu(app, ui));
                if icon_button(
                    ui,
                    "maskPanelArrange",
                    Icon::Sidebar,
                    vec2(24.0, 24.0),
                    floating,
                    true,
                    if floating { "Dock to panel" } else { "Float panel" },
                )
                .clicked()
                {
                    let _ = app.run("ui.maskPanel", json!({"floating": !floating}));
                }
            });
        });
        creation_status(app, ui);
        // Reserve the 24 pt footer, 6 pt separator and the two inter-item gaps.
        // Filling the entire offered content size includes the window's title spacing again
        // and causes the outer frame to grow on each repaint.
        let footer = 24.0 + 6.0 + 2.0 * ui.spacing().item_spacing.y;
        let height = if floating { (ui.available_height() - footer).max(100.0) } else { 240.0 };
        egui::ScrollArea::vertical().id_salt(("mask-list", floating)).max_height(height).auto_shrink([false, !floating]).show(ui, |ui| {
            for (index, mask) in d.masks.iter().enumerate() {
                ui.push_id(mask.id, |ui| mask_row(app, ui, mask, index, d.masks.len()));
            }
        });
        ui.separator();
        overlay_footer(app, ui);
    });
    register(ui.ctx(), if floating { "maskList:floating" } else { "maskList:embedded" }, response.response.rect);
}

fn mask_row(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &lightcraft_develop::Mask, index: usize, count: usize) {
    let t = Tokens::get(ui.ctx());
    let selected = app.session.active_mask == Some(m.id);
    egui::Frame::NONE.fill(if selected { t.inset } else { egui::Color32::TRANSPARENT }).corner_radius(4.0).show(ui, |ui| {
        if let Some((rid, name)) = app.ui.renaming_mask.as_mut().filter(|(rid, _)| *rid == m.id) {
            let rid = *rid;
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width()).id_salt(("maskRename", rid)));
            register(ui.ctx(), format!("maskRename:{rid}"), r.rect);
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.ui.renaming_mask = None;
            } else if r.lost_focus() {
                let name = name.trim().to_string();
                app.ui.renaming_mask = None;
                if !name.is_empty() && name != m.name {
                    let _ = app.run("mask.rename", json!({"id": rid, "name": name}));
                }
            }
        } else {
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
            register(ui.ctx(), format!("mask:{}", m.id), r);
            if resp.hovered() {
                ui.painter().rect_filled(r, 4.0, t.hover.gamma_multiply(0.5));
            }
            let thumb = Rect::from_min_size(r.min + vec2(4.0, 5.0), vec2(38.0, 38.0));
            ui.painter().rect_filled(thumb, 3.0, t.chrome);
            ui.painter().rect_stroke(
                thumb,
                3.0,
                Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { t.accent } else { t.text_dim }),
                egui::StrokeKind::Inside,
            );
            let icon = m.components.first().map(|c| kind_label(&c.shape).1).unwrap_or(Icon::Mask);
            paint(ui.painter(), thumb.shrink(8.0), icon, t.text_label);
            let color = if m.visible { t.text } else { t.text_disabled };
            let g = one_line(ui.painter(), &m.name, t.font(13.0), color, (r.width() - 102.0).max(0.0));
            let elided = g.elided;
            let at = pos2(r.left() + 50.0, r.center().y - g.size().y / 2.0);
            register(ui.ctx(), format!("maskName:{}", m.id), Rect::from_min_size(at, g.size()));
            ui.painter().galley(at, g, color);
            let resp = if elided { resp.on_hover_text(&m.name) } else { resp };
            let eye_rect = Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), vec2(22.0, 24.0));
            let eye = ui.interact(eye_rect, ui.id().with("visible"), Sense::click());
            register(ui.ctx(), format!("maskVisible:{}", m.id), eye_rect);
            paint(ui.painter(), eye_rect.shrink(4.0), if m.visible { Icon::Eye } else { Icon::EyeOff }, t.text_dim);
            if eye.clicked() {
                let _ = app.run("mask.visible", json!({"id": m.id}));
            }
            let more_rect = eye_rect.translate(vec2(-24.0, 0.0));
            let more = ui.interact(more_rect, ui.id().with("options"), Sense::click());
            register(ui.ctx(), format!("maskMenu:{}", m.id), more_rect);
            paint(ui.painter(), more_rect.shrink(4.0), Icon::More, t.text_dim);
            egui::Popup::menu(&more).show(|ui| mask_menu(app, ui, m.id, &m.name, m.visible, index, count));
            if resp.clicked() {
                let _ = app.run("mask.select", json!({"id": m.id}));
            }
            if resp.double_clicked() {
                app.ui.renaming_mask = Some((m.id, m.name.clone()));
            }
            resp.context_menu(|ui| mask_menu(app, ui, m.id, &m.name, m.visible, index, count));
        }
        if selected {
            egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 6, top: 0, bottom: 6 }).show(ui, |ui| {
                for (i, c) in m.components.iter().enumerate() {
                    component_row(app, ui, m, i, c);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let size = vec2(((ui.available_width() - 8.0) / 2.0).max(1.0), 32.0);
                    for (op, label, key, alias) in [("add", "Add", "maskPlus", "maskAddComp"), ("subtract", "Subtract", "maskMinus", "maskSubComp")] {
                        let label = crate::i18n::tr(label);
                        let text = one_line(ui.painter(), label, t.semibold(12.5), t.text, (size.x - 16.0).max(0.0));
                        let button = ui
                            .add_sized(size, egui::Button::new(text).stroke(Stroke::new(1.0, t.button_border)).corner_radius(4.0))
                            .on_hover_text(label);
                        register(ui.ctx(), format!("button:{key}:{}", m.id), button.rect);
                        register(ui.ctx(), format!("button:{alias}"), button.rect);
                        egui::Popup::menu(&button).show(|ui| component_menu(app, ui, op));
                    }
                });
                describe_field(app, ui, false);
            });
        }
    });
}

fn component_row(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &lightcraft_develop::Mask, i: usize, c: &lightcraft_develop::MaskComponent) {
    let t = Tokens::get(ui.ctx());
    let (kind, icon) = kind_label(&c.shape);
    let label = c.name.clone().unwrap_or_else(|| match &c.shape {
        MaskShape::Prompt { text, .. } => format!("“{text}”"),
        _ => {
            let number = m.components.iter().take(i.saturating_add(1)).filter(|c| kind_label(&c.shape).0 == kind).count();
            format!("{} {number}", crate::i18n::tr(kind))
        }
    });
    if let Some((_, _, name)) = app.ui.renaming_component.as_mut().filter(|(mid, k, _)| *mid == m.id && *k == i) {
        let r = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width()).id_salt(("compRename", m.id, i)));
        register(ui.ctx(), format!("componentRename:{i}"), r.rect);
        if !r.has_focus() && !r.lost_focus() {
            r.request_focus();
        }
        if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
            app.ui.renaming_component = None;
        } else if r.lost_focus() {
            let name = name.trim().to_string();
            app.ui.renaming_component = None;
            if Some(&name) != c.name.as_ref() {
                let _ = app.run("mask.component", json!({"id": m.id, "component": i, "action": "rename", "name": name}));
            }
        }
        return;
    }
    let selected = component(app, m) == Some(i);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    register(ui.ctx(), format!("component:{i}"), r);
    ui.painter().rect_filled(
        r,
        3.0,
        if selected {
            t.tool_active
        } else if resp.hovered() {
            t.hover
        } else {
            egui::Color32::TRANSPARENT
        },
    );
    paint(ui.painter(), Rect::from_min_size(r.min + vec2(5.0, 6.0), vec2(20.0, 20.0)), icon, t.text_label);
    let op = match c.op {
        lightcraft_develop::MaskOp::Add => "",
        lightcraft_develop::MaskOp::Subtract => "− ",
        lightcraft_develop::MaskOp::Intersect => "∩ ",
    };
    let text = format!("{op}{label}{}", if c.invert { crate::i18n::tr(" (inverted)") } else { "" });
    let g = one_line(ui.painter(), &text, t.font(12.5), t.text_label, (r.width() - 62.0).max(0.0));
    ui.painter().galley(pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, t.text_label);
    let more_rect = Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), vec2(24.0, 24.0));
    let more = ui.interact(more_rect, ui.id().with(("componentMenu", i)), Sense::click());
    register(ui.ctx(), format!("button:componentMenu{i}"), more_rect);
    paint(ui.painter(), more_rect.shrink(4.0), Icon::More, t.text_dim);
    egui::Popup::menu(&more).show(|ui| component_row_menu(app, ui, m.id, i, &label, m.components.len()));
    if resp.clicked() {
        let _ = app.run("mask.selectComponent", json!({"id": m.id, "component": i}));
    }
    if resp.double_clicked() {
        app.ui.renaming_component = Some((m.id, i, label.clone()));
    }
    resp.on_hover_text(&text).context_menu(|ui| component_row_menu(app, ui, m.id, i, &label, m.components.len()));
}

fn overlay_footer(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let mut overlay = app.ui.mask_overlay;
        let t = Tokens::get(ui.ctx());
        let label = crate::i18n::tr("Show overlay (O)");
        let galley = one_line(ui.painter(), label, t.font(12.5), t.text_label, (ui.available_width() - 84.0).max(0.0));
        let check = ui.checkbox(&mut overlay, galley).on_hover_text(label);
        register(ui.ctx(), "icon:maskOverlay", check.rect);
        if check.changed() {
            let _ = app.run("view.maskOverlay", json!({"show": overlay}));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let options = icon_button(ui, "maskOverlayOptions", Icon::More, vec2(24.0, 24.0), false, true, "Overlay");
            egui::Popup::menu(&options).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                ui.set_width(250.0);
                overlay_options(app, ui);
            });
            let (r, swatch) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            register(ui.ctx(), "maskOverlaySwatch", r);
            let [red, green, blue] = app.ui.mask_overlay_color;
            ui.painter().circle_filled(r.center(), 8.0, egui::Color32::from_rgb(red, green, blue));
            ui.painter().circle_stroke(r.center(), 8.0, Stroke::new(1.0, Tokens::get(ui.ctx()).text_dim));
            egui::Popup::menu(&swatch).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                ui.set_width(250.0);
                overlay_options(app, ui);
            });
        });
    });
}
