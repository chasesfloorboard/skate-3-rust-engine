//! Material for dynamic props: character-style probe + sun lighting (see
//! prop_material.wgsl). Prop pieces spawn with the generic PBR material and are
//! switched to this one once their StandardMaterial is available.
use bevy::{asset::embedded_asset, prelude::*, render::render_resource::AsBindGroup, shader::ShaderRef};
use std::collections::HashMap;

/// Meshes under an entity with this (vehicles) are shaded like props.
#[derive(Component)]
pub(crate) struct ProbeLit;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct PropMaterial {
    /// rgb base colour, a alpha cutoff (-1 when opaque).
    #[uniform(0)]
    pub params: Vec4,
    #[texture(1)]
    #[sampler(2)]
    pub diffuse: Option<Handle<Image>>,
    #[storage(3, read_only)]
    pub frame: Handle<bevy::render::storage::ShaderStorageBuffer>,
    /// x: 1 for metal, which reflects `matcap` (a sphere map) by view normal;
    /// y: 1 for alpha-blended, whose alpha cannot carry the lamp share.
    #[uniform(4)]
    pub shine: Vec4,
    #[texture(5)]
    #[sampler(6)]
    pub matcap: Option<Handle<Image>>,
    pub alpha: AlphaMode,
}
impl Material for PropMaterial {
    fn fragment_shader() -> ShaderRef {
        bevy::asset::AssetPath::from(bevy::asset::embedded_path!("prop_material.wgsl")).with_source("embedded").into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        self.alpha
    }
    // The default prepass shader cannot see the prop texture, so it wrote
    // depth over cut-out texels (nets, slats) and hid everything behind them,
    // leaving the clear colour showing through as a solid pale sheet.
    fn enable_prepass() -> bool {
        false
    }
}

/// Keeps the imported dynamic_lights.wgsl module loaded.
#[derive(Resource)]
struct DynamicLightsShader(#[allow(dead_code)] Handle<Shader>);

pub(crate) struct PropMaterialPlugin;
impl Plugin for PropMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "prop_material.wgsl");
        // Shared lamp lighting (characters, props), imported by path.
        embedded_asset!(app, "dynamic_lights.wgsl");
        let lights: Handle<Shader> = bevy::asset::load_embedded_asset!(app.world().resource::<AssetServer>(), "dynamic_lights.wgsl");
        app.insert_resource(DynamicLightsShader(lights));
        app.add_plugins(MaterialPlugin::<PropMaterial>::default())
            .add_systems(Update, adopt);
    }
}

fn adopt(
    mut commands: Commands,
    pieces: Query<(Entity, &MeshMaterial3d<StandardMaterial>, Has<crate::props::PropPiece>)>,
    lit: Query<(), With<ProbeLit>>,
    parents: Query<&ChildOf>,
    standard: Res<Assets<StandardMaterial>>,
    images: Res<Assets<Image>>,
    mut materials: ResMut<Assets<PropMaterial>>,
    mut cache: Local<HashMap<AssetId<StandardMaterial>, Handle<PropMaterial>>>,
) {
    for (entity, material, piece) in &pieces {
        if !piece && !parents.iter_ancestors(entity).any(|a| lit.contains(a)) { continue; }
        let Some(source) = standard.get(material) else { continue };
        // Wait for the texture: its alpha decides the blend mode below.
        if source.base_color_texture.as_ref().is_some_and(|t| images.get(t).is_none()) { continue; }
        let handle = cache.entry(material.id()).or_insert_with(|| {
            // Some prop materials arrive flagged opaque although their
            // textures carry cut-out alpha (bench slats); alpha-test those
            // too, all at retail's ALPHAREF 30 rather than the portable 0.5.
            let alpha = match source.alpha_mode {
                AlphaMode::Opaque if source.base_color_texture.as_ref()
                    .and_then(|t| images.get(t)).is_some_and(cut_out) => AlphaMode::Mask(30. / 255.),
                AlphaMode::Mask(_) => AlphaMode::Mask(30. / 255.),
                other => other,
            };
            let cutoff = if let AlphaMode::Mask(c) = alpha { c } else { -1.0 };
            materials.add(PropMaterial {
                params: source.base_color.to_linear().to_vec3().extend(cutoff),
                diffuse: source.base_color_texture.clone(),
                frame: crate::retail_render::FRAME_BUFFER,
                // Metallic materials carry their reflection sphere map in the
                // emissive slot (tools/mk8_to_mixamo.py).
                shine: Vec4::new(if source.metallic > 0.5 && source.emissive_texture.is_some() { 1. } else { 0. },
                                 if matches!(alpha, AlphaMode::Blend | AlphaMode::Premultiplied | AlphaMode::Add) { 1. } else { 0. }, 0., 0.),
                matcap: source.emissive_texture.clone().filter(|_| source.metallic > 0.5),
                alpha,
            })
        }).clone();
        commands.entity(entity).remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(handle));
    }
}

/// An 8-bit RGBA texture whose alpha is a cut-out mask: a real share of fully
/// clear texels and few in-between ones (alpha used for anything else, such as
/// a specular mask, is mostly in-between).
fn cut_out(image: &Image) -> bool {
    use bevy::render::render_resource::TextureFormat;
    if !matches!(image.texture_descriptor.format, TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm) { return false; }
    let Some(data) = image.data.as_ref() else { return false };
    let (mut clear, mut partial, mut total) = (0usize, 0usize, 0usize);
    // First mip only (the level-0 texels come first).
    let texels = (image.width() * image.height()) as usize;
    for texel in data.chunks_exact(4).take(texels) {
        total += 1;
        match texel[3] {
            0..30 => clear += 1,
            30..226 => partial += 1,
            _ => {}
        }
    }
    total > 0 && clear * 50 > total && partial * 5 < total
}
