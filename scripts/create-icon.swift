import Foundation
import CoreGraphics
import ImageIO
import UniformTypeIdentifiers

let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let folder = root.appendingPathComponent("assets/AppIcon.iconset")
try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
let teal = CGColor(red: 15/255, green: 118/255, blue: 110/255, alpha: 1)
let mint = CGColor(red: 167/255, green: 225/255, blue: 194/255, alpha: 1)
for (points, scale) in [(16,1),(16,2),(32,1),(32,2),(128,1),(128,2),(256,1),(256,2),(512,1),(512,2)] {
    let size = points * scale
    let ctx = CGContext(data: nil, width: size, height: size, bitsPerComponent: 8,
        bytesPerRow: size * 4, space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.scaleBy(x: CGFloat(size)/1024, y: CGFloat(size)/1024)
    let body = CGPath(roundedRect: CGRect(x: 80, y: 80, width: 864, height: 864), cornerWidth: 194, cornerHeight: 194, transform: nil)
    ctx.setShadow(offset: CGSize(width: 0, height: -14), blur: 24, color: CGColor(gray: 0, alpha: 0.18))
    ctx.setFillColor(teal); ctx.addPath(body); ctx.fillPath()
    ctx.setShadow(offset: .zero, blur: 0)
    ctx.setStrokeColor(CGColor(gray: 1, alpha: 0.95)); ctx.setLineWidth(24); ctx.setLineCap(.round)
    ctx.addPath(CGPath(roundedRect: CGRect(x: 310, y: 310, width: 404, height: 404), cornerWidth: 66, cornerHeight: 66, transform: nil)); ctx.strokePath()
    for n in [-1,0,1] {
        let offset = CGFloat(n) * 108
        for side in [-1.0,1.0] {
            let a = CGFloat(side)
            ctx.move(to: CGPoint(x: 512+offset, y: 512+202*a)); ctx.addLine(to: CGPoint(x: 512+offset, y: 512+286*a))
            ctx.move(to: CGPoint(x: 512+202*a, y: 512+offset)); ctx.addLine(to: CGPoint(x: 512+286*a, y: 512+offset))
        }
    }
    ctx.strokePath(); ctx.setFillColor(mint); ctx.fillEllipse(in: CGRect(x: 464, y: 464, width: 96, height: 96))
    let name = "icon_\(points)x\(points)" + (scale == 2 ? "@2x" : "") + ".png"
    let target = folder.appendingPathComponent(name)
    let output = CGImageDestinationCreateWithURL(target as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(output, ctx.makeImage()!, nil)
    guard CGImageDestinationFinalize(output) else { fatalError("Could not write icon") }
}
