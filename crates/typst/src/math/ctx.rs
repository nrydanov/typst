use std::f64::consts::SQRT_2;

use comemo::Prehashed;
use ecow::EcoString;
use rustybuzz::Feature;
use ttf_parser::gsub::{AlternateSubstitution, SingleSubstitution, SubstitutionSubtable};
use ttf_parser::math::MathValue;
use ttf_parser::opentype_layout::LayoutTable;
use ttf_parser::GlyphId;
use unicode_math_class::MathClass;
use unicode_segmentation::UnicodeSegmentation;

use crate::diag::SourceResult;
use crate::engine::Engine;
use crate::foundations::{Content, NativeElement, Smart, StyleChain, Styles};
use crate::layout::{Abs, Axes, BoxElem, Em, Frame, Layout, Regions, Size};
use crate::math::{
    FrameFragment, GlyphFragment, LayoutMath, MathFragment, MathRow, MathSize, MathStyle,
    MathVariant, THICK,
};
use crate::model::ParElem;
use crate::realize::realize;
use crate::syntax::{is_newline, Span};
use crate::text::{
    features, variant, BottomEdge, BottomEdgeMetric, Font, FontStyle, FontWeight,
    TextElem, TextSize, TopEdge, TopEdgeMetric,
};

macro_rules! scaled {
    ($ctx:expr, text: $text:ident, display: $display:ident $(,)?) => {
        match $ctx.style.size {
            MathSize::Display => scaled!($ctx, $display),
            _ => scaled!($ctx, $text),
        }
    };
    ($ctx:expr, $name:ident) => {
        $ctx.constants.$name().scaled($ctx)
    };
}

macro_rules! percent {
    ($ctx:expr, $name:ident) => {
        $ctx.constants.$name() as f64 / 100.0
    };
}

/// The context for math layout.
pub struct MathContext<'a, 'b, 'v> {
    pub engine: &'v mut Engine<'b>,
    pub regions: Regions<'static>,
    pub font: &'a Font,
    pub ttf: &'a ttf_parser::Face<'a>,
    pub table: ttf_parser::math::Table<'a>,
    pub constants: ttf_parser::math::Constants<'a>,
    pub ssty_table: Option<ttf_parser::gsub::AlternateSubstitution<'a>>,
    pub glyphwise_tables: Option<Vec<GlyphwiseSubsts<'a>>>,
    pub space_width: Em,
    pub fragments: Vec<MathFragment>,
    pub local: Styles,
    pub style: MathStyle,
    pub size: Abs,
    outer: StyleChain<'a>,
    style_stack: Vec<(MathStyle, Abs)>,
}

impl<'a, 'b, 'v> MathContext<'a, 'b, 'v> {
    pub fn new(
        engine: &'v mut Engine<'b>,
        styles: StyleChain<'a>,
        regions: Regions,
        font: &'a Font,
        block: bool,
    ) -> Self {
        let math_table = font.ttf().tables().math.unwrap();
        let gsub_table = font.ttf().tables().gsub;
        let constants = math_table.constants.unwrap();

        let ssty_table = gsub_table
            .and_then(|gsub| {
                gsub.features
                    .find(ttf_parser::Tag::from_bytes(b"ssty"))
                    .and_then(|feature| feature.lookup_indices.get(0))
                    .and_then(|index| gsub.lookups.get(index))
            })
            .and_then(|ssty| ssty.subtables.get::<SubstitutionSubtable>(0))
            .and_then(|ssty| match ssty {
                SubstitutionSubtable::Alternate(alt_glyphs) => Some(alt_glyphs),
                _ => None,
            });

        let features = features(styles);
        let glyphwise_tables = gsub_table.map(|gsub| {
            features
                .into_iter()
                .filter_map(|feature| GlyphwiseSubsts::new(gsub, feature))
                .collect()
        });

        let size = TextElem::size_in(styles);
        let ttf = font.ttf();
        let space_width = ttf
            .glyph_index(' ')
            .and_then(|id| ttf.glyph_hor_advance(id))
            .map(|advance| font.to_em(advance))
            .unwrap_or(THICK);

        let variant = variant(styles);
        Self {
            engine,
            regions: Regions::one(regions.base(), Axes::splat(false)),
            font,
            ttf: font.ttf(),
            table: math_table,
            constants,
            ssty_table,
            glyphwise_tables,
            space_width,
            fragments: vec![],
            local: Styles::new(),
            style: MathStyle {
                variant: MathVariant::Serif,
                size: if block { MathSize::Display } else { MathSize::Text },
                class: Smart::Auto,
                cramped: false,
                bold: variant.weight >= FontWeight::BOLD,
                italic: match variant.style {
                    FontStyle::Normal => Smart::Auto,
                    FontStyle::Italic | FontStyle::Oblique => Smart::Custom(true),
                },
            },
            size,
            outer: styles,
            style_stack: vec![],
        }
    }

    pub fn push(&mut self, fragment: impl Into<MathFragment>) {
        self.fragments.push(fragment.into());
    }

    pub fn extend(&mut self, fragments: Vec<MathFragment>) {
        self.fragments.extend(fragments);
    }

    pub fn layout_root(&mut self, elem: &dyn LayoutMath) -> SourceResult<MathRow> {
        let row = self.layout_fragments(elem)?;
        Ok(MathRow::new(row))
    }

    pub fn layout_fragment(
        &mut self,
        elem: &dyn LayoutMath,
    ) -> SourceResult<MathFragment> {
        let row = self.layout_fragments(elem)?;
        Ok(MathRow::new(row).into_fragment(self))
    }

    pub fn layout_fragments(
        &mut self,
        elem: &dyn LayoutMath,
    ) -> SourceResult<Vec<MathFragment>> {
        let prev = std::mem::take(&mut self.fragments);
        elem.layout_math(self)?;
        Ok(std::mem::replace(&mut self.fragments, prev))
    }

    pub fn layout_row(&mut self, elem: &dyn LayoutMath) -> SourceResult<MathRow> {
        let fragments = self.layout_fragments(elem)?;
        Ok(MathRow::new(fragments))
    }

    pub fn layout_frame(&mut self, elem: &dyn LayoutMath) -> SourceResult<Frame> {
        Ok(self.layout_fragment(elem)?.into_frame())
    }

    pub fn layout_box(&mut self, boxed: &BoxElem) -> SourceResult<Frame> {
        Ok(boxed
            .layout(self.engine, self.outer.chain(&self.local), self.regions)?
            .into_frame())
    }

    pub fn layout_content(&mut self, content: &Content) -> SourceResult<Frame> {
        Ok(content
            .layout(self.engine, self.outer.chain(&self.local), self.regions)?
            .into_frame())
    }

    pub fn layout_text(&mut self, elem: &TextElem) -> SourceResult<MathFragment> {
        let text = elem.text();
        let span = elem.span();
        let mut chars = text.chars();
        let fragment = if let Some(mut glyph) = chars
            .next()
            .filter(|_| chars.next().is_none())
            .map(|c| self.style.styled_char(c))
            .and_then(|c| GlyphFragment::try_new(self, c, span))
        {
            // A single letter that is available in the math font.
            match self.style.size {
                MathSize::Script => {
                    glyph.make_scriptsize(self);
                }
                MathSize::ScriptScript => {
                    glyph.make_scriptscriptsize(self);
                }
                _ => (),
            }

            let class = self.style.class.as_custom().or(glyph.class);
            if class == Some(MathClass::Large) {
                let mut variant = if self.style.size == MathSize::Display {
                    let height = scaled!(self, display_operator_min_height)
                        .max(SQRT_2 * glyph.height());
                    glyph.stretch_vertical(self, height, Abs::zero())
                } else {
                    glyph.into_variant()
                };
                // TeXbook p 155. Large operators are always vertically centered on the axis.
                variant.center_on_axis(self);
                variant.into()
            } else {
                glyph.into()
            }
        } else if text.chars().all(|c| c.is_ascii_digit() || c == '.') {
            // Numbers aren't that difficult.
            let mut fragments = vec![];
            for c in text.chars() {
                let c = self.style.styled_char(c);
                fragments.push(GlyphFragment::new(self, c, span).into());
            }
            let frame = MathRow::new(fragments).into_frame(self);
            FrameFragment::new(self, frame).with_text_like(true).into()
        } else {
            // Anything else is handled by Typst's standard text layout.
            let mut style = self.style;
            if self.style.italic == Smart::Auto {
                style = style.with_italic(false);
            }
            let text: EcoString = text.chars().map(|c| style.styled_char(c)).collect();
            if text.contains(is_newline) {
                let mut fragments = vec![];
                for (i, piece) in text.split(is_newline).enumerate() {
                    if i != 0 {
                        fragments.push(MathFragment::Linebreak);
                    }
                    if !piece.is_empty() {
                        fragments.push(self.layout_complex_text(piece, span)?.into());
                    }
                }
                let mut frame = MathRow::new(fragments).into_frame(self);
                let axis = scaled!(self, axis_height);
                frame.set_baseline(frame.height() / 2.0 + axis);
                FrameFragment::new(self, frame).into()
            } else {
                self.layout_complex_text(&text, span)?.into()
            }
        };
        Ok(fragment)
    }

    pub fn layout_complex_text(
        &mut self,
        text: &str,
        span: Span,
    ) -> SourceResult<FrameFragment> {
        let spaced = text.graphemes(true).nth(1).is_some();
        let elem = TextElem::packed(text)
            .styled(TextElem::set_top_edge(TopEdge::Metric(TopEdgeMetric::Bounds)))
            .styled(TextElem::set_bottom_edge(BottomEdge::Metric(
                BottomEdgeMetric::Bounds,
            )))
            .spanned(span);

        // There isn't a natural width for a paragraph in a math environment;
        // because it will be placed somewhere probably not at the left margin
        // it will overflow.  So emulate an `hbox` instead and allow the paragraph
        // to extend as far as needed.
        let span = elem.span();
        let frame = ParElem::new(vec![Prehashed::new(elem)])
            .spanned(span)
            .layout(
                self.engine,
                self.outer.chain(&self.local),
                false,
                Size::splat(Abs::inf()),
                false,
            )?
            .into_frame();

        Ok(FrameFragment::new(self, frame)
            .with_class(MathClass::Alphabetic)
            .with_text_like(true)
            .with_spaced(spaced))
    }

    pub fn styles(&self) -> StyleChain {
        self.outer.chain(&self.local)
    }

    pub fn realize(&mut self, content: &Content) -> SourceResult<Option<Content>> {
        realize(self.engine, content, self.outer.chain(&self.local))
    }

    pub fn style(&mut self, style: MathStyle) {
        self.style_stack.push((self.style, self.size));
        let base_size = TextElem::size_in(self.styles()) / self.style.size.factor(self);
        self.size = base_size * style.size.factor(self);
        self.local.set(TextElem::set_size(TextSize(self.size.into())));
        self.local
            .set(TextElem::set_style(if style.italic == Smart::Custom(true) {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            }));
        self.local.set(TextElem::set_weight(if style.bold {
            FontWeight::BOLD
        } else {
            // The normal weight is what we started with.
            // It's 400 for CM Regular, 450 for CM Book.
            self.font.info().variant.weight
        }));
        self.style = style;
    }

    pub fn unstyle(&mut self) {
        (self.style, self.size) = self.style_stack.pop().unwrap();
        self.local.unset();
        self.local.unset();
        self.local.unset();
    }
}

pub(super) trait Scaled {
    fn scaled(self, ctx: &MathContext) -> Abs;
}

impl Scaled for i16 {
    fn scaled(self, ctx: &MathContext) -> Abs {
        ctx.font.to_em(self).scaled(ctx)
    }
}

impl Scaled for u16 {
    fn scaled(self, ctx: &MathContext) -> Abs {
        ctx.font.to_em(self).scaled(ctx)
    }
}

impl Scaled for Em {
    fn scaled(self, ctx: &MathContext) -> Abs {
        self.at(ctx.size)
    }
}

impl Scaled for MathValue<'_> {
    fn scaled(self, ctx: &MathContext) -> Abs {
        self.value.scaled(ctx)
    }
}

/// An OpenType substitution table that is applicable to glyph-wise substitutions.
pub enum GlyphwiseSubsts<'a> {
    Single(SingleSubstitution<'a>),
    Alternate(AlternateSubstitution<'a>, u32),
}

impl<'a> GlyphwiseSubsts<'a> {
    pub fn new(gsub: LayoutTable<'a>, feature: Feature) -> Option<Self> {
        let table = gsub
            .features
            .find(ttf_parser::Tag(feature.tag.0))
            .and_then(|feature| feature.lookup_indices.get(0))
            .and_then(|index| gsub.lookups.get(index))?;
        let table = table.subtables.get::<SubstitutionSubtable>(0)?;
        match table {
            SubstitutionSubtable::Single(single_glyphs) => {
                Some(Self::Single(single_glyphs))
            }
            SubstitutionSubtable::Alternate(alt_glyphs) => {
                Some(Self::Alternate(alt_glyphs, feature.value))
            }
            _ => None,
        }
    }

    pub fn try_apply(&self, glyph_id: GlyphId) -> Option<GlyphId> {
        match self {
            Self::Single(single) => match single {
                SingleSubstitution::Format1 { coverage, delta } => coverage
                    .get(glyph_id)
                    .map(|_| GlyphId(glyph_id.0.wrapping_add(*delta as u16))),
                SingleSubstitution::Format2 { coverage, substitutes } => {
                    coverage.get(glyph_id).and_then(|idx| substitutes.get(idx))
                }
            },
            Self::Alternate(alternate, value) => alternate
                .coverage
                .get(glyph_id)
                .and_then(|idx| alternate.alternate_sets.get(idx))
                .and_then(|set| set.alternates.get(*value as u16)),
        }
    }

    pub fn apply(&self, glyph_id: GlyphId) -> GlyphId {
        self.try_apply(glyph_id).unwrap_or(glyph_id)
    }
}
