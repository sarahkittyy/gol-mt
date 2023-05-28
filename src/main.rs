use kiss3d::{self as k3d, window::Window, nalgebra::Point3};
use std::time;

mod gol;

fn main() {
    let mut window = Window::new_with_size("gol3d-mt", 1920, 1080);
    window.set_background_color(0.2, 0.2, 0.2);

    // clock for timing steps
    let mut clock = time::Duration::ZERO;
    let mut t = time::Instant::now();
    let timestep = time::Duration::from_millis(250);

    let root: &mut k3d::scene::SceneNode = window.scene_mut();
    let mut map_render_node = root.add_group();
    let mut game = gol::GameOfLife::new(3, 25, map_render_node.clone(), gol::GolRules {
        percent_alive: 0.3,
        kill: Box::new(|alive| {
            ![5,6,7, 14].contains(&alive)
        }),
        resurrect: Box::new(|alive| {
            alive == 6
        }),
        wrap_around: true,
        diagonal_neighbors: true,
    });

    let mut camera = k3d::camera::ArcBall::new(Point3::new(-5.0, 40.0, -5.0), Point3::new(12.0, 12.0, 12.0));

    window.set_light(k3d::light::Light::StickToCamera);

    while window.render_with_camera(&mut camera) {
        let now = time::Instant::now();
        let dt = now - t;
        t = now;
        clock += dt;

        while clock >= timestep {
            clock -= timestep;
            game.step();
            println!("alive: {}", game.alive());
        }
    }
}
