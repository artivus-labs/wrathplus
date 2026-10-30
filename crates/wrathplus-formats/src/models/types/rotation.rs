/// Shortest-arc slerp on plain arrays (`wrathplus-formats` carries no math crate).
pub(super) fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let mut dot = (0..4).map(|i| a[i] * b[i]).sum::<f32>();
    let mut b = b;
    if dot < 0.0 {
        b = [-b[0], -b[1], -b[2], -b[3]];
        dot = -dot;
    }
    let (w0, w1) = if dot > 0.9995 {
        (1.0 - t, t) // near-parallel: lerp, then renormalise below
    } else {
        let theta = dot.clamp(-1.0, 1.0).acos();
        let sin = theta.sin();
        (((1.0 - t) * theta).sin() / sin, (t * theta).sin() / sin)
    };
    let mut q = [0.0f32; 4];
    for i in 0..4 {
        q[i] = a[i] * w0 + b[i] * w1;
    }
    let len = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if len > 0.0 {
        for c in &mut q {
            *c /= len;
        }
    }
    q
}
