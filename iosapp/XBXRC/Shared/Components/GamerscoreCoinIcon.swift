import SwiftUI

struct GamerscoreCoinIcon: View {
    var body: some View {
        ZStack {
            Circle()
                .fill(.green)
            Text("G")
                .font(.system(size: 9, weight: .black, design: .rounded))
                .foregroundStyle(.white)
        }
        .frame(width: 18, height: 18)
        .accessibilityHidden(true)
    }
}
