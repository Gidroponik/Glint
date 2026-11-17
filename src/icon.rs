// Общий код для приложения и build.rs (подключается через include!).

/// Рисует значок приложения программно: белая плашка-«лист» (два противоположных угла
/// сильно скруглены) с чёрными уголками кадра. Строго монохромно, без градиентов.
pub fn icon_rgba(n: usize) -> Vec<u8> {
    let mut px = vec![0u8; n * n * 4];
    let nf = n as f32;
    let pad = nf * 0.04;
    let half = nf / 2.0 - pad;
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (cx, cy) = (fx - nf / 2.0, fy - nf / 2.0);
            // левый верхний и правый нижний углы — большие, два других — маленькие
            let big = (cx < 0.0) == (cy < 0.0);
            let r = nf * if big { 0.36 } else { 0.1 };
            let dx = cx.abs() - (half - r);
            let dy = cy.abs() - (half - r);
            let d = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r;
            let a = (0.5 - d).clamp(0.0, 1.0);
            let mut c = 255.0f32;
            let (u, v) = (fx / nf, fy / nf);
            let (lo, hi, th, len) = (0.29, 0.71, 0.085, 0.17);
            let near = |p: f32, edge: f32| (p - edge).abs() < th / 2.0;
            let within = |p: f32| {
                (p > lo - th / 2.0 && p < lo + len) || (p < hi + th / 2.0 && p > hi - len)
            };
            let bracket = ((near(u, lo) || near(u, hi)) && within(v))
                || ((near(v, lo) || near(v, hi)) && within(u));
            if bracket {
                c = 0.0;
            }
            let i = (y * n + x) * 4;
            px[i] = c as u8;
            px[i + 1] = c as u8;
            px[i + 2] = c as u8;
            px[i + 3] = (a * 255.0) as u8;
        }
    }
    px
}
