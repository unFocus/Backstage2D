package backstage2d.display.spritesheets 
{
	import backstage2d.display.*;
	import flash.geom.Point;
	import flash.geom.Rectangle;
	
	/**
	 * Arranges rectangles to pack them tightly in a single rectangle.
	 * @author Kevin Newman
	 */
	public class SpriteNode 
	{
		public var left:SpriteNode;
		public var right:SpriteNode;
		public var rect:Rectangle;
		public var regOffset:Point = new Point();
		
		protected var leaf:Boolean// = false;
		//public var actor:Actor;
		
		public function insert( image:Rectangle ):SpriteNode
		{
			// if is a leaf, return
			if ( leaf ) {
				return null;
			}
			// if node is not a leaf and has child nodes, pass it on
			else if ( left ) {
				return left.insert( image ) || right.insert( image );
			}
			
			// If image doesn't fit in current rect at all, return null.
			if ( rect.width < image.width || rect.height < image.height ) {
				return null;
			}
			
			// fits just right - make a leaf
			if ( rect.width == image.width && rect.height == image.height ) {
				leaf = true;
				return this;
			}
			
			// It fits, but not perfectly, so we make smaller rects.
			left = new SpriteNode();
			right = new SpriteNode();
			
			var dw: Number = rect.width - image.width;
			var dh: Number = rect.height - image.height;
			
			// divides a wider rect vertically
			if ( dw > dh )
			{
				// left is the same position as parent, but less wide
				left.rect = new Rectangle(
					rect.x, rect.y,
					image.width, rect.height
				);
				// right is the rest of parent
				right.rect = new Rectangle(
					rect.x + image.width,
					rect.y,
					rect.width - image.width,
					rect.height
				);
			}
			else
			{
				left.rect = new Rectangle(
					rect.x, rect.y,
					rect.width,
					image.height
				);
				right.rect = new Rectangle(
					rect.x,
					rect.y + image.height,
					rect.width,
					rect.height - image.height
				);
			}
			
			// try the left space
			return left.insert( image );
		}
		
	}
}
