package backstage2d.display.classic 
{
	import backstage2d.display.Animation;
	import backstage2d.display.spritesheets.*;
	
	import flash.display.MovieClip;
	import flash.display.BitmapData;
	import flash.geom.Matrix;
	
	import backstage2d.namespaces.bs2d;
	use namespace bs2d;
	
	/**
	 * ...
	 * @author Kevin Newman
	 */
	public class BSMovieClip extends Animation
	{
		protected var clip:MovieClip;
		
		public var quality:String = StageQuality.BEST;
		
		public function BSMovieClip( clip:MovieClip, frameRate:Number = 20 ) 
		{
			super( clip.width, clip.height, frameRate );
			this.clip = clip;
			frameNodes = new Vector.<SpriteNode>(clip.framesTotal, true);
		}
		
		bs2d var frameNodes:Vector.<SpriteNode> = new <SpriteNode>[];
		
		override bs2d function set texNode( node:SpriteNode ):void {
			SpriteRegistry.instance[ sprite ] = node;
		}
		override bs2d function get texNode():SpriteNode {
			return SpriteRegistry.instance[ sprite ];
		}
		
		/**
		 * Needs to be called for each frame of the animation.
		 * @return
		 */
		public override function draw( canvas:BitmapData, transform:Matrix = null ):BitmapData
		{
			clip.gotoAndStop( i );
			canvas.draw( data, transform, null, null, null, true );
			
			return canvas;
		}
		
	}
}
